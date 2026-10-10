use std::sync::Mutex;

use serde::Serialize;

const IMPORT_LINK_PREFIX: &str = "agentmanager://import";

/// Delivery identities belong to an opening, not to its URL: opening the same
/// link again after dismissing the dialog must still work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ImportLink {
    id: u64,
    url: String,
}

#[derive(Default)]
struct CurrentLinks {
    next_id: u64,
    links: Vec<ImportLink>,
}

#[derive(Default)]
pub(crate) struct ImportLinkState(Mutex<CurrentLinks>);

impl ImportLinkState {
    pub(crate) fn record(&self, urls: Vec<String>, initial: bool) -> Vec<ImportLink> {
        let mut state = self.0.lock().unwrap();
        // A runtime event may arrive while setup reads the plugin's initial
        // snapshot. Never overwrite that newer event with the older snapshot.
        if initial && state.next_id != 0 {
            return Vec::new();
        }
        let links: Vec<_> = urls
            .into_iter()
            .filter(|url| url.starts_with(IMPORT_LINK_PREFIX) && url.len() <= 64 * 1024)
            .map(|url| {
                state.next_id += 1;
                ImportLink {
                    id: state.next_id,
                    url,
                }
            })
            .collect();
        if !links.is_empty() {
            state.links = links.clone();
        }
        links
    }

    pub(crate) fn current(&self) -> Vec<ImportLink> {
        self.0.lock().unwrap().links.clone()
    }
}

#[tauri::command]
pub(crate) fn import_links_current(state: tauri::State<'_, ImportLinkState>) -> Vec<ImportLink> {
    state.current()
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "agentmanager://import?data=example";

    #[test]
    fn cold_start_snapshot_and_runtime_delivery_share_ids() {
        let state = ImportLinkState::default();
        let cold = state.record(vec![URL.into()], true);
        assert_eq!(state.current(), cold);
        let warm = state.record(vec![URL.into()], false);
        assert_eq!(state.current(), warm);
        assert_ne!(cold[0].id, warm[0].id);
        assert_eq!(cold[0].url, warm[0].url);
    }

    #[test]
    fn runtime_open_wins_over_setup_snapshot() {
        let state = ImportLinkState::default();
        let warm = state.record(vec![format!("{URL}-new")], false);
        assert!(state.record(vec![URL.into()], true).is_empty());
        assert_eq!(state.current(), warm);
    }

    #[test]
    fn unrelated_and_oversized_links_are_not_delivered() {
        let state = ImportLinkState::default();
        let links = state.record(
            vec![
                "https://example.com".into(),
                format!("{URL}{}", "a".repeat(64 * 1024)),
                URL.into(),
            ],
            true,
        );
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, URL);
    }
}
