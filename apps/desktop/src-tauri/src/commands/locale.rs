//! The operating system's preferred languages, from which the frontend
//! picks the app's language when it follows the system (docs/i18n.md).
//!
//! `sys_locale::get_locales` reads the macOS preferred-languages list
//! (`CFLocaleCopyPreferredLanguages`), the Windows user UI languages
//! (`GetUserPreferredUILanguages`) and, on Linux, `LANGUAGE`, `LC_ALL`,
//! `LC_MESSAGES` and `LANG`. Nothing else (no IP address, no time zone)
//! decides the language. The choice among them is the frontend's
//! (`resolveLocale` in `src/i18n/resolve.ts`); this only tidies the list.

/// The app's effective language as the agent should know it (protocol 13,
/// `setLocale`): set by the main window when it starts and whenever the
/// language changes, sent to a running agent at once and to every agent
/// the bridge connects to later (`agent::subscribe`). Plugin texts, the
/// texts the agent draws on keys and its tray then follow it.
#[derive(Debug, Default)]
pub struct AgentLocale(std::sync::Mutex<Option<String>>);

impl AgentLocale {
    /// Stores `tag`; whether it changed.
    pub fn set(&self, tag: &str) -> bool {
        let mut current = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current.as_deref() == Some(tag) {
            return false;
        }
        *current = Some(tag.to_owned());
        true
    }

    /// The language last set, if any.
    pub fn get(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Tells the agent the app's effective language (`en`, `pl`). Only the
/// tag crosses: never the system's other locale settings. An agent of
/// protocol 12 or older is not told (it keeps English); a stopped agent is
/// told when it runs again.
#[tauri::command]
pub async fn set_agent_locale(
    state: tauri::State<'_, crate::state::AppState>,
    locale: tauri::State<'_, AgentLocale>,
    tag: String,
) -> Result<(), String> {
    if !deckpilot_ipc::is_locale_tag(&tag) {
        return Err(format!("`{tag}` is not a language tag"));
    }
    if !locale.set(&tag) {
        return Ok(());
    }
    let paths = state.paths.clone();
    if let Ok(mut client) =
        deckpilot_ipc::AgentClient::connect(&paths, deckpilot_ipc::ClientKind::Desktop).await
        && let Err(error) = client.set_locale(&tag).await
    {
        eprintln!("the agent was not told the language: {error}");
    }
    Ok(())
}

/// The system's preferred languages as BCP 47 tags, most preferred first.
#[tauri::command]
pub fn system_locales() -> Vec<String> {
    normalize_locales(sys_locale::get_locales())
}

/// Turns POSIX locale names into BCP 47 tags (`pl_PL.UTF-8` → `pl-PL`),
/// drops the "no language" locales (`C`, `POSIX`) and empty entries, and
/// keeps the first occurrence of each tag.
fn normalize_locales(locales: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for locale in locales {
        // `LANGUAGE` may hold a colon-separated list.
        for part in locale.split(':') {
            let name = part.split(['.', '@']).next().unwrap_or_default().trim();
            if name.is_empty()
                || name.eq_ignore_ascii_case("c")
                || name.eq_ignore_ascii_case("posix")
            {
                continue;
            }
            let tag = name.replace('_', "-");
            if !tags.iter().any(|known| known.eq_ignore_ascii_case(&tag)) {
                tags.push(tag);
            }
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::normalize_locales;

    fn normalized(locales: &[&str]) -> Vec<String> {
        normalize_locales(locales.iter().map(ToString::to_string))
    }

    #[test]
    fn keeps_bcp47_tags_in_order() {
        assert_eq!(normalized(&["pl-PL", "en-GB"]), ["pl-PL", "en-GB"]);
    }

    #[test]
    fn converts_posix_names() {
        assert_eq!(
            normalized(&["pl_PL.UTF-8", "de_DE@euro"]),
            ["pl-PL", "de-DE"]
        );
    }

    #[test]
    fn the_agent_locale_remembers_the_last_tag() {
        let locale = super::AgentLocale::default();
        assert_eq!(locale.get(), None);
        assert!(locale.set("pl"));
        assert!(!locale.set("pl"), "unchanged");
        assert!(locale.set("en"));
        assert_eq!(locale.get().as_deref(), Some("en"));
    }

    #[test]
    fn splits_language_lists_and_drops_duplicates() {
        assert_eq!(
            normalized(&["pl:en_US", "pl_PL", "en-US"]),
            ["pl", "en-US", "pl-PL"]
        );
    }

    #[test]
    fn drops_c_posix_and_empty_locales() {
        assert_eq!(
            normalized(&["C", "POSIX", "C.UTF-8", "", "  "]),
            Vec::<String>::new()
        );
    }
}
