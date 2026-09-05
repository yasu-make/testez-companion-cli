use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use config::Config;
use console::style;
use dashmap::DashMap;
use inquire::Select;
use place::{
    format_places_human, immediate_place_match, resolve_place, snapshot_state_places, PlaceInfo,
    PlaceMatchError,
};
use state::AppState;
use std::{
    io::{stdin, IsTerminal},
    net::SocketAddr,
    process::exit,
    sync::Arc,
    time::Duration,
};
use tokio::{
    fs::read_to_string,
    spawn,
    sync::Mutex,
    time::{sleep, Instant},
};

mod api;
mod config;
mod place;
mod state;
mod testez;

/// Exit code for no places, empty `--list`, or `--place` match errors.
const EXIT_SELECTION: i32 = 2;

#[derive(Parser, Debug)]
#[command(
    about = "CLI for TestEZ Companion",
    after_help = "Exit codes:\n  \
        0  Success (--list found places, or tests passed)\n  \
        1  Tests ran and one or more failed\n  \
        2  No places checked in, --list was empty, --place did not uniquely match,\n     \
           or interactive selection failed (non-TTY / prompt error)"
)]
struct Cli {
    /// Only print failing tests (existing behavior)
    #[arg(long)]
    pub only_print_failures: bool,

    /// List connected places and exit without selecting one or running tests
    #[arg(long)]
    pub list: bool,

    /// Select a place by GUID, name, or numeric id (never prompts)
    #[arg(long, value_name = "GUID|NAME|ID", conflicts_with = "list")]
    pub place: Option<String>,

    /// How long to wait for places to check in, in seconds
    #[arg(long, default_value_t = 5)]
    pub timeout: u64,

    /// Machine-readable JSON for --list and the final test summary
    #[arg(long)]
    pub json: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let config: Arc<Config> = {
        let contents = read_to_string("testez-companion.toml")
            .await
            .expect("Missing testez-companion.toml");

        Arc::new(toml::from_str(&contents).expect("Failed to parse testez-companion.toml"))
    };

    let state = Arc::new(AppState {
        config,
        places: DashMap::new(),
        active_place: Mutex::new(None),
        only_log_failures: cli.only_print_failures,
        json: cli.json,
    });

    let state_clone = Arc::clone(&state);
    spawn(async move {
        run_place_flow(Arc::clone(&state_clone), cli).await;
        // Successful --place / interactive selection returns so /poll can run tests.
        // Any other return would leave axum listening on :28859.
        if state_clone.active_place.lock().await.is_none() {
            eprintln!(
                "{}",
                style("Place selection ended without activating a place.").red()
            );
            exit(EXIT_SELECTION);
        }
    });

    let app = Router::new()
        .route("/poll", get(api::poll))
        .route("/logs", post(api::logs))
        .route("/results", post(api::results))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 28859));
    axum_server::bind(addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn run_place_flow(state: Arc<AppState>, cli: Cli) {
    let timeout = Duration::from_secs(cli.timeout);

    if cli.list {
        wait_for_timeout(timeout).await;
        list_places_and_exit(&state, cli.json);
    } else if let Some(query) = cli.place {
        wait_and_select_place(state, timeout, &query, cli.json).await;
    } else {
        wait_and_select_interactive(state, timeout).await;
    }
}

/// Wait the full timeout so every Studio instance can check in via `/poll`.
async fn wait_for_timeout(timeout: Duration) {
    let start_time = Instant::now();
    // Give the HTTP server a moment to bind before plugins start polling.
    sleep(Duration::from_millis(200)).await;
    eprintln!("{}", style("Waiting for place(s) to check in...").dim());

    while start_time.elapsed() < timeout {
        sleep(Duration::from_millis(100)).await;
    }
}

fn list_places_and_exit(state: &AppState, json: bool) -> ! {
    let places = snapshot_state_places(state);
    print_place_list(&places, json);

    if places.is_empty() {
        eprintln!(
            "{}",
            style("No places have reported anything. Studio might not be open?").red()
        );
        exit(EXIT_SELECTION);
    }

    exit(0);
}

async fn wait_and_select_place(state: Arc<AppState>, timeout: Duration, query: &str, json: bool) {
    let start_time = Instant::now();
    sleep(Duration::from_millis(200)).await;
    eprintln!("{}", style("Waiting for place(s) to check in...").dim());

    loop {
        let places = snapshot_state_places(&state);
        let timed_out = start_time.elapsed() >= timeout;

        if let Some(result) = immediate_place_match(&places, query) {
            match result {
                Ok(place) => {
                    activate_place(&state, place).await;
                    return;
                }
                Err(error) => {
                    print_place_match_error(query, error, &places, json);
                    exit(EXIT_SELECTION);
                }
            }
        }

        if timed_out {
            // Name matches (and anything that is not a stable GUID/id) resolve once.
            match resolve_place(&places, query) {
                Ok(place) => {
                    activate_place(&state, place).await;
                    return;
                }
                Err(error) => {
                    print_place_match_error(query, error, &places, json);
                    exit(EXIT_SELECTION);
                }
            }
        }

        sleep(Duration::from_millis(100)).await;
    }
}

/// How the default (no `--place`) path should treat the current place set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractiveChoice {
    WaitForMore,
    AutoSelect,
    Prompt,
    ExitNonTty,
}

fn decide_interactive_selection(place_count: usize, stdin_is_tty: bool) -> InteractiveChoice {
    match place_count {
        0 => InteractiveChoice::WaitForMore,
        1 => InteractiveChoice::AutoSelect,
        _ if stdin_is_tty => InteractiveChoice::Prompt,
        _ => InteractiveChoice::ExitNonTty,
    }
}

async fn wait_and_select_interactive(state: Arc<AppState>, timeout: Duration) {
    let start_time = Instant::now();
    sleep(Duration::from_secs(1)).await;

    eprintln!("{}", style("Waiting for place(s) to check in...").dim());

    loop {
        if start_time.elapsed() > timeout {
            eprintln!(
                "{}",
                style("No places have reported anything. Studio might not be open?").red()
            );
            exit(EXIT_SELECTION);
        }

        let stdin_is_tty = stdin().is_terminal();
        let key: Option<String> =
            match decide_interactive_selection(state.places.len(), stdin_is_tty) {
                InteractiveChoice::WaitForMore => None,
                InteractiveChoice::AutoSelect => {
                    Some(state.places.iter().next().unwrap().key().to_string())
                }
                InteractiveChoice::Prompt => Some(inquire_place(Arc::clone(&state))),
                InteractiveChoice::ExitNonTty => {
                    exit_non_tty_multi_place(&snapshot_state_places(&state));
                }
            };

        match key {
            Some(key) => {
                eprintln!(
                    "{}",
                    style(format!("Waiting for results from place {}...", key)).dim(),
                );
                state.active_place.lock().await.replace(key);
                break;
            }
            None => {
                sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn activate_place(state: &AppState, place: &PlaceInfo) {
    eprintln!(
        "{}",
        style(format!("Waiting for results from place {}...", place.guid)).dim(),
    );
    state.active_place.lock().await.replace(place.guid.clone());
}

fn print_place_list(places: &[PlaceInfo], json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(places).expect("Failed to serialize place list")
        );
    } else if places.is_empty() {
        // Human-readable empty list is implied by the stderr error.
    } else {
        println!("{}", format_places_human(places));
    }
}

fn print_place_match_error(
    query: &str,
    error: PlaceMatchError,
    all_places: &[PlaceInfo],
    json: bool,
) {
    match error {
        PlaceMatchError::NotFound => {
            if all_places.is_empty() {
                eprintln!(
                    "{}",
                    style("No places have reported anything. Studio might not be open?").red()
                );
            } else {
                eprintln!(
                    "{}",
                    style(format!("No place matched '--place {query}'")).red()
                );
            }
        }
        PlaceMatchError::Ambiguous(matches) => {
            eprintln!(
                "{}",
                style(format!(
                    "'--place {query}' is ambiguous; {} places matched",
                    matches.len()
                ))
                .red()
            );
        }
    }

    eprint_connected_places(all_places);

    if json {
        println!(
            "{}",
            serde_json::to_string(all_places).expect("Failed to serialize place list")
        );
    }
}

fn eprint_connected_places(places: &[PlaceInfo]) {
    if places.is_empty() {
        return;
    }
    eprintln!("{}", style("Connected places:").dim());
    for line in format_places_human(places).lines() {
        eprintln!("  {line}");
    }
}

fn pass_place_flag_hint() -> String {
    "Pass --place <guid|name|id> to select a place without a prompt.".to_string()
}

fn exit_non_tty_multi_place(places: &[PlaceInfo]) -> ! {
    eprintln!(
        "{}",
        style("stdin is not a TTY; cannot prompt to choose among multiple places.").red()
    );
    eprint_connected_places(places);
    eprintln!("{}", style(pass_place_flag_hint()).dim());
    exit(EXIT_SELECTION);
}

fn exit_prompt_failure(error: impl std::fmt::Display, places: &[PlaceInfo]) -> ! {
    eprintln!(
        "{}",
        style(format!("Failed to prompt for place selection: {error}")).red()
    );
    eprint_connected_places(places);
    eprintln!("{}", style(pass_place_flag_hint()).dim());
    exit(EXIT_SELECTION);
}

fn inquire_place(state: Arc<AppState>) -> String {
    let places = snapshot_state_places(&state);
    // Defense in depth: skip inquire entirely when stdin is not a TTY so
    // a NotTTY prompt error cannot unwind the selection task while axum lives.
    if !stdin().is_terminal() {
        exit_non_tty_multi_place(&places);
    }

    let options: Vec<String> = places
        .iter()
        .map(|place| format!("{} ({}) [{}]", place.name, place.id, place.guid))
        .collect();

    let selected = match Select::new("Select a place to run tests on:", options).prompt() {
        Ok(selected) => selected,
        Err(error) => exit_prompt_failure(error, &places),
    };

    let key = selected
        .split_whitespace()
        .last()
        .unwrap()
        .trim_matches(|c| c == '[' || c == ']');

    key.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_flags() {
        let cli = Cli::try_parse_from([
            "testez-companion-cli",
            "--list",
            "--json",
            "--timeout",
            "10",
            "--only-print-failures",
        ])
        .unwrap();
        assert!(cli.list);
        assert!(cli.json);
        assert!(cli.only_print_failures);
        assert_eq!(cli.timeout, 10);
        assert!(cli.place.is_none());
    }

    #[test]
    fn default_timeout_is_five() {
        let cli = Cli::try_parse_from(["testez-companion-cli"]).unwrap();
        assert_eq!(cli.timeout, 5);
        assert!(!cli.list);
        assert!(!cli.json);
        assert!(cli.place.is_none());
    }

    #[test]
    fn list_and_place_conflict() {
        let err =
            Cli::try_parse_from(["testez-companion-cli", "--list", "--place", "x"]).unwrap_err();
        assert!(err.to_string().contains("cannot be used with"));
    }

    #[test]
    fn parses_place() {
        let cli = Cli::try_parse_from(["testez-companion-cli", "--place", "Lobby"]).unwrap();
        assert_eq!(cli.place.as_deref(), Some("Lobby"));
        assert!(!cli.list);
    }

    #[test]
    fn non_tty_with_multiple_places_exits_without_prompt() {
        assert_eq!(
            decide_interactive_selection(2, false),
            InteractiveChoice::ExitNonTty
        );
        assert_eq!(
            decide_interactive_selection(3, false),
            InteractiveChoice::ExitNonTty
        );
    }

    #[test]
    fn tty_with_multiple_places_prompts() {
        assert_eq!(
            decide_interactive_selection(2, true),
            InteractiveChoice::Prompt
        );
    }

    #[test]
    fn single_place_auto_selects_without_tty() {
        assert_eq!(
            decide_interactive_selection(1, false),
            InteractiveChoice::AutoSelect
        );
        assert_eq!(
            decide_interactive_selection(1, true),
            InteractiveChoice::AutoSelect
        );
    }

    #[test]
    fn zero_places_keeps_waiting() {
        assert_eq!(
            decide_interactive_selection(0, false),
            InteractiveChoice::WaitForMore
        );
        assert_eq!(
            decide_interactive_selection(0, true),
            InteractiveChoice::WaitForMore
        );
    }

    #[test]
    fn non_tty_hint_mentions_place_flag() {
        assert!(pass_place_flag_hint().contains("--place"));
    }

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState {
            config: Arc::new(Config {
                roots: vec![],
                test_extra_options: None,
            }),
            places: DashMap::new(),
            active_place: Mutex::new(None),
            only_log_failures: false,
            json: false,
        })
    }

    #[tokio::test]
    async fn name_query_waits_full_timeout_before_activating() {
        let state = test_state();
        state.places.insert(
            "guid-a".to_string(),
            state::Place {
                name: "Lobby".to_string(),
                id: 1,
            },
        );

        let timeout = Duration::from_millis(300);
        let start = Instant::now();
        wait_and_select_place(Arc::clone(&state), timeout, "Lobby", false).await;
        assert!(
            start.elapsed() >= timeout,
            "name matches must wait the full timeout, elapsed {:?}",
            start.elapsed()
        );
        assert_eq!(state.active_place.lock().await.as_deref(), Some("guid-a"));
    }

    #[tokio::test]
    async fn guid_query_activates_before_timeout() {
        let state = test_state();
        state.places.insert(
            "guid-a".to_string(),
            state::Place {
                name: "Lobby".to_string(),
                id: 1,
            },
        );

        let timeout = Duration::from_secs(5);
        let start = Instant::now();
        wait_and_select_place(Arc::clone(&state), timeout, "guid-a", false).await;
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "exact GUID should early-select, elapsed {:?}",
            start.elapsed()
        );
        assert_eq!(state.active_place.lock().await.as_deref(), Some("guid-a"));
    }

    #[tokio::test]
    async fn unique_id_activates_before_timeout() {
        let state = test_state();
        state.places.insert(
            "guid-a".to_string(),
            state::Place {
                name: "Lobby".to_string(),
                id: 42,
            },
        );

        let timeout = Duration::from_secs(5);
        let start = Instant::now();
        wait_and_select_place(Arc::clone(&state), timeout, "42", false).await;
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "unique numeric id should early-select, elapsed {:?}",
            start.elapsed()
        );
        assert_eq!(state.active_place.lock().await.as_deref(), Some("guid-a"));
    }
}
