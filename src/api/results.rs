use crate::{
    state::AppState,
    testez::{ReporterChildNode, ReporterOutput, ReporterStatus},
};
use axum::{extract::State, http::StatusCode, Json};
use console::style;
use serde::Serialize;
use serde_json::Value;
use std::{fmt::Write, process::exit, sync::Arc, time::Duration};
use tokio::{spawn, time::sleep};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestSummary {
    success: bool,
    success_count: u32,
    failure_count: u32,
    skipped_count: u32,
}

fn emit_line(json: bool, line: impl AsRef<str>) {
    if json {
        eprintln!("{}", line.as_ref());
    } else {
        println!("{}", line.as_ref());
    }
}

fn emit(json: bool, text: impl AsRef<str>) {
    if json {
        eprint!("{}", text.as_ref());
    } else {
        print!("{}", text.as_ref());
    }
}

fn print_children(state: &Arc<AppState>, children: Vec<ReporterChildNode>, indent: u32) -> bool {
    let mut success = true;

    for child in children {
        if state.only_log_failures && child.status != ReporterStatus::Failure {
            continue;
        }

        let styled_phrase = match child.status {
            ReporterStatus::Success => style(format!("✓ {}", child.plan_node.phrase)).green(),
            ReporterStatus::Failure => {
                success = false;
                style(format!("X {}", child.plan_node.phrase)).red()
            }
            ReporterStatus::Skipped => style(format!("↪ {}", child.plan_node.phrase)).blue(),
        };
        emit_line(
            state.json,
            format!("{}{}", " ".repeat(indent as usize), styled_phrase),
        );

        for error in child.errors {
            let indented_error: String = error.split('\n').fold(String::new(), |mut acc, line| {
                writeln!(
                    acc,
                    "{:indent$}{}",
                    " ",
                    line,
                    indent = (indent + 2) as usize
                )
                .unwrap();
                acc
            });
            emit(state.json, indented_error);
        }

        if !print_children(state, child.children, indent + 2) {
            success = false;
        }
    }
    success
}

pub async fn results(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> StatusCode {
    let output: ReporterOutput =
        serde_json::from_value(body).expect("Failed to parse JSON from plugin");

    let success = print_children(&state, output.children, 0);

    emit_line(state.json, "");
    emit_line(
        state.json,
        format!("{} {}", style("✓ Success:").green(), output.success_count),
    );
    emit_line(
        state.json,
        format!("{} {}", style("X Failure:").red(), output.failure_count),
    );
    emit_line(
        state.json,
        format!("{} {}", style("↪ Skip:").blue(), output.skipped_count),
    );

    if state.json {
        let summary = TestSummary {
            success,
            success_count: output.success_count,
            failure_count: output.failure_count,
            skipped_count: output.skipped_count,
        };
        println!(
            "{}",
            serde_json::to_string(&summary).expect("Failed to serialize test summary")
        );
    }

    // This is mildly cursed - we need to return a status code, but we also need
    // to exit the progam so that we don't keep receiving results.
    spawn(async move {
        sleep(Duration::from_millis(100)).await;

        exit(if success { 0 } else { 1 });
    });

    StatusCode::OK
}
