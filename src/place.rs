use crate::state::{AppState, Place};
use dashmap::DashMap;
use serde::Serialize;

/// Machine-readable place record used by `--list --json` and matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlaceInfo {
    pub guid: String,
    pub name: String,
    pub id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceMatchError {
    NotFound,
    Ambiguous(Vec<PlaceInfo>),
}

/// Snapshot connected places, sorted by GUID for stable output.
pub fn snapshot_places(places: &DashMap<String, Place>) -> Vec<PlaceInfo> {
    let mut list: Vec<PlaceInfo> = places
        .iter()
        .map(|entry| PlaceInfo {
            guid: entry.key().clone(),
            name: entry.value().name.clone(),
            id: entry.value().id,
        })
        .collect();
    list.sort_by(|a, b| a.guid.cmp(&b.guid));
    list
}

pub fn snapshot_state_places(state: &AppState) -> Vec<PlaceInfo> {
    snapshot_places(&state.places)
}

/// Format places for humans, one per line: `name (id) [guid]`.
pub fn format_places_human(places: &[PlaceInfo]) -> String {
    places
        .iter()
        .map(|place| format!("{} ({}) [{}]", place.name, place.id, place.guid))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Resolve `--place` against connected places.
///
/// Matching order:
/// 1. Exact GUID (DashMap key / `place-guid` header)
/// 2. Unique place id (`place-id` formatted as a decimal string)
/// 3. Unique place name, case-sensitive
/// 4. Unique place name, ASCII case-insensitive
///
/// Zero or multiple matches after a step that produced candidates is an error.
pub fn resolve_place<'a>(
    places: &'a [PlaceInfo],
    query: &str,
) -> Result<&'a PlaceInfo, PlaceMatchError> {
    if let Some(place) = places.iter().find(|place| place.guid == query) {
        return Ok(place);
    }

    let id_matches: Vec<&PlaceInfo> = places
        .iter()
        .filter(|place| place.id.to_string() == query)
        .collect();
    match id_matches.as_slice() {
        [place] => return Ok(place),
        [] => {}
        _ => {
            return Err(PlaceMatchError::Ambiguous(
                id_matches.into_iter().cloned().collect(),
            ))
        }
    }

    let exact_name: Vec<&PlaceInfo> = places.iter().filter(|place| place.name == query).collect();
    match exact_name.as_slice() {
        [place] => return Ok(place),
        [] => {}
        _ => {
            return Err(PlaceMatchError::Ambiguous(
                exact_name.into_iter().cloned().collect(),
            ))
        }
    }

    let ci_name: Vec<&PlaceInfo> = places
        .iter()
        .filter(|place| place.name.eq_ignore_ascii_case(query))
        .collect();
    match ci_name.as_slice() {
        [place] => Ok(place),
        [] => Err(PlaceMatchError::NotFound),
        _ => Err(PlaceMatchError::Ambiguous(
            ci_name.into_iter().cloned().collect(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(guid: &str, name: &str, id: u64) -> PlaceInfo {
        PlaceInfo {
            guid: guid.to_string(),
            name: name.to_string(),
            id,
        }
    }

    #[test]
    fn prefers_exact_guid() {
        let places = vec![place("guid-a", "Lobby", 1), place("guid-b", "guid-a", 2)];
        let matched = resolve_place(&places, "guid-a").unwrap();
        assert_eq!(matched.guid, "guid-a");
        assert_eq!(matched.name, "Lobby");
    }

    #[test]
    fn matches_unique_place_id() {
        let places = vec![place("guid-a", "Lobby", 123), place("guid-b", "Arena", 456)];
        let matched = resolve_place(&places, "456").unwrap();
        assert_eq!(matched.guid, "guid-b");
    }

    #[test]
    fn prefers_id_over_name() {
        let places = vec![place("guid-a", "456", 123), place("guid-b", "Arena", 456)];
        let matched = resolve_place(&places, "456").unwrap();
        assert_eq!(matched.guid, "guid-b");
    }

    #[test]
    fn matches_case_sensitive_name() {
        let places = vec![place("guid-a", "Lobby", 1), place("guid-b", "lobby", 2)];
        let matched = resolve_place(&places, "Lobby").unwrap();
        assert_eq!(matched.guid, "guid-a");
    }

    #[test]
    fn falls_back_to_case_insensitive_name() {
        let places = vec![place("guid-a", "Lobby", 1), place("guid-b", "Arena", 2)];
        let matched = resolve_place(&places, "lobby").unwrap();
        assert_eq!(matched.guid, "guid-a");
    }

    #[test]
    fn ambiguous_case_insensitive_name() {
        let places = vec![place("guid-a", "Lobby", 1), place("guid-b", "lobby", 2)];
        let err = resolve_place(&places, "LOBBY").unwrap_err();
        assert!(matches!(err, PlaceMatchError::Ambiguous(list) if list.len() == 2));
    }

    #[test]
    fn ambiguous_duplicate_names() {
        let places = vec![place("guid-a", "Lobby", 1), place("guid-b", "Lobby", 2)];
        let err = resolve_place(&places, "Lobby").unwrap_err();
        assert!(matches!(err, PlaceMatchError::Ambiguous(list) if list.len() == 2));
    }

    #[test]
    fn not_found() {
        let places = vec![place("guid-a", "Lobby", 1)];
        assert_eq!(
            resolve_place(&places, "missing"),
            Err(PlaceMatchError::NotFound)
        );
    }

    #[test]
    fn empty_list_is_not_found() {
        assert_eq!(resolve_place(&[], "x"), Err(PlaceMatchError::NotFound));
    }

    #[test]
    fn formats_human_list() {
        let places = vec![place("abc", "Lobby", 10)];
        assert_eq!(format_places_human(&places), "Lobby (10) [abc]");
    }

    #[test]
    fn json_shape() {
        let places = vec![place("abc", "Lobby", 10)];
        let json = serde_json::to_string(&places).unwrap();
        assert_eq!(json, r#"[{"guid":"abc","name":"Lobby","id":10}]"#);
    }
}
