//! The texts DeckPilot itself draws on keys and in the tray, in the app's
//! languages (English and Polish, `docs/i18n.md`).
//!
//! The desktop app translates its interface with i18next; the agent has no
//! React and no JavaScript, so the few texts it produces on its own have a
//! small catalog here, under stable keys. A key face remembers which of its
//! texts came from this catalog ([`FaceText`]); the agent puts them into
//! the effective language just before drawing ([`KeyFace::localize`]), so
//! a change of language redraws exactly those keys. Texts the user wrote
//! (key titles, action and layout names) carry no key and are never
//! touched; nor are a plugin's state labels, which the plugin produces in
//! the language it is given.

use std::fmt;

use crate::face::KeyFace;

/// The languages of the catalog. Every other language falls back to
/// English, like the app's interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Language {
    #[default]
    En,
    Pl,
}

impl Language {
    /// The language for a BCP 47 tag (`pl`, `pl-PL`, `pl_PL`): its
    /// language subtag if the catalog has it, else English.
    pub fn from_tag(tag: &str) -> Self {
        let language = tag.split(['-', '_']).next().unwrap_or_default();
        if language.eq_ignore_ascii_case("pl") {
            Self::Pl
        } else {
            Self::En
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Pl => "pl",
        }
    }

    pub const ALL: [Self; 2] = [Self::En, Self::Pl];
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

macro_rules! texts {
    ($($variant:ident => $key:literal, $en:literal, $pl:literal;)*) => {
        /// A text of the catalog, by stable key.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum SystemText {
            $($variant,)*
        }

        impl SystemText {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// The stable key (`face.back`).
            pub fn key(self) -> &'static str {
                match self {
                    $(Self::$variant => $key,)*
                }
            }

            /// The text in `language`.
            pub fn text(self, language: Language) -> &'static str {
                match (self, language) {
                    $((Self::$variant, Language::En) => $en,
                    (Self::$variant, Language::Pl) => $pl,)*
                }
            }

            /// The text of a key.
            pub fn from_key(key: &str) -> Option<Self> {
                match key {
                    $($key => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

texts! {
    // Key faces.
    Back => "face.back", "Back", "Wstecz";
    Folder => "face.folder", "Folder", "Folder";
    Workflow => "face.workflow", "Workflow", "Przepływ";
    Missing => "face.missing", "Missing", "Brak akcji";
    Unavailable => "face.unavailable", "Unavailable", "Niedostępne";
    NotInThisProfile => "face.notInThisProfile", "Not in this profile", "Nie w tym profilu";
    Error => "face.error", "Error", "Błąd";
    NoPreviousLayout => "face.noPreviousLayout", "No previous layout", "Brak poprzedniego układu";
    NoProfile => "face.noProfile", "No profile", "Brak profilu";
    NoLayout => "face.noLayout", "No layout", "Brak układu";
    // The tray.
    TrayOpen => "tray.open", "Open DeckPilot", "Otwórz DeckPilot";
    TrayOpenToUnlock => "tray.openToUnlock", "Open DeckPilot to Unlock…", "Otwórz DeckPilot, aby odblokować…";
    TrayActiveProfile => "tray.activeProfile", "Active Profile", "Aktywny profil";
    TrayNoProfiles => "tray.noProfiles", "No profiles", "Brak profili";
    TrayProject => "tray.project", "Project: {key} – {name}", "Projekt: {key} – {name}";
    TrayDevices => "tray.devices", "Connected Devices", "Podłączone urządzenia";
    TrayNoDevices => "tray.noDevices", "No devices", "Brak urządzeń";
    TrayConnected => "tray.connected", "connected", "połączone";
    TrayDisconnected => "tray.disconnected", "disconnected", "rozłączone";
    TrayRefresh => "tray.refresh", "Refresh Devices", "Odśwież urządzenia";
    TrayAbout => "tray.about", "About DeckPilot", "O programie DeckPilot";
    TrayQuit => "tray.quit", "Quit DeckPilot", "Zakończ DeckPilot";
    TrayLockedKeyUnavailable => "tray.lockedKeyUnavailable", "Locked – the key could not be read from the credential store", "Zablokowano – nie udało się odczytać klucza z magazynu poświadczeń";
    TrayLocked => "tray.locked", "Locked – actions are paused", "Zablokowano – akcje są wstrzymane";
    TraySetUp => "tray.setUp", "Set up DeckPilot to start", "Skonfiguruj DeckPilot, aby zacząć";
    TrayEncrypt => "tray.encrypt", "Encrypt your configuration to continue", "Zaszyfruj konfigurację, aby kontynuować";
}

/// The default names of built-in action types that a key shows as its
/// title while the user gave it none (`deck` module), by action type id:
/// English, Polish.
const ACTION_NAMES: &[(&str, &str, &str)] = &[
    ("deck.next-page", "Next page", "Następna strona"),
    ("deck.previous-page", "Previous page", "Poprzednia strona"),
    ("deck.page-indicator", "Page indicator", "Wskaźnik strony"),
    ("deck.switch-profile", "Switch profile", "Przełącz profil"),
    ("deck.switch-layout", "Switch layout", "Przełącz układ"),
    (
        "deck.previous-layout",
        "Back to previous layout",
        "Wróć do poprzedniego układu",
    ),
    ("deck.brightness", "Brightness", "Jasność"),
    ("deck.random", "Random action", "Losowa akcja"),
    (
        "deck.multi-switch",
        "Multi Action Switch",
        "Przełącznik wielu akcji",
    ),
];

/// The name of the built-in action type `action_type` in `language`, if
/// the catalog has it.
pub fn action_type_name(action_type: &str, language: Language) -> Option<&'static str> {
    ACTION_NAMES
        .iter()
        .find(|(id, ..)| *id == action_type)
        .map(|(_, en, pl)| match language {
            Language::En => *en,
            Language::Pl => *pl,
        })
}

/// Whether `title` is one of the catalog's names of `action_type`, in any
/// language (a default title, not one the user wrote).
pub fn is_action_type_name(action_type: &str, title: &str) -> bool {
    ACTION_NAMES
        .iter()
        .any(|(id, en, pl)| *id == action_type && (title == *en || title == *pl))
}

/// Where a face's title came from, when not from the user.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TitleSource {
    /// A text of the catalog.
    System(SystemText),
    /// The action's name, which the action instance took from its type
    /// when it was created (no label of its own): a default name the
    /// type's owner (the catalog, a plugin's translations) may give in
    /// another language. A name the user changed matches none of them and
    /// is kept.
    ActionName { action_type: String },
}

/// Which texts of a [`KeyFace`] came from a catalog rather than from the
/// user, so the agent can draw them in the effective language. Not part
/// of what the face looks like: faces compare and hash equal whatever it
/// says, and it is never serialized.
#[derive(Debug, Clone, Default)]
pub struct FaceText {
    pub title: Option<TitleSource>,
    pub detail: Option<SystemText>,
    /// The title is one the user wrote for a plugin's action (the key's or
    /// the action's label): the plugin's dynamic title does not replace it
    /// ([`KeyFace::with_state`], `docs/key-text.md`).
    pub user_title: bool,
}

impl PartialEq for FaceText {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for FaceText {}

impl std::hash::Hash for FaceText {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl FaceText {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.detail.is_none()
    }
}

impl KeyFace {
    /// A face whose title is the catalog's `text` (in English until
    /// [`KeyFace::localize`]).
    pub fn system(style: crate::FaceStyle, text: SystemText) -> Self {
        let mut face = Self::new(style, text.text(Language::En));
        face.text.title = Some(TitleSource::System(text));
        face
    }

    /// The face with the catalog's `text` as its secondary line.
    #[must_use]
    pub fn with_system_detail(mut self, text: SystemText) -> Self {
        self.detail = Some(text.text(Language::En).to_owned());
        self.text.detail = Some(text);
        self
    }

    /// Marks the title as the action's default name, taken from the type
    /// `action_type`.
    #[must_use]
    pub fn with_action_name(mut self, action_type: impl Into<String>) -> Self {
        self.text.title = Some(TitleSource::ActionName {
            action_type: action_type.into(),
        });
        self
    }

    /// Whether any text of the face comes from a catalog.
    pub fn has_translated_text(&self) -> bool {
        !self.text.is_empty()
    }

    /// Puts the catalog's texts of the face into `language`. An action's
    /// default name is looked up in the built-in catalog, else with
    /// `names` (`action type`, `title as shown` → the name in `language`,
    /// or `None` when the title is not one of the type's default names).
    /// Texts the user wrote are left alone.
    pub fn localize(&mut self, language: Language, names: &dyn Fn(&str, &str) -> Option<String>) {
        match &self.text.title {
            Some(TitleSource::System(text)) => text.text(language).clone_into(&mut self.title),
            Some(TitleSource::ActionName { action_type }) => {
                if is_action_type_name(action_type, &self.title) {
                    if let Some(name) = action_type_name(action_type, language) {
                        name.clone_into(&mut self.title);
                    }
                } else if let Some(name) = names(action_type, &self.title) {
                    self.title = name;
                }
            }
            None => {}
        }
        if let Some(text) = self.text.detail {
            self.detail = Some(text.text(language).to_owned());
        }
    }
}

/// Expands `{name}` placeholders of a catalog text (the tray's project
/// line).
pub fn format(text: SystemText, language: Language, args: &[(&str, &str)]) -> String {
    let mut out = text.text(language).to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FaceStyle;

    #[test]
    fn every_text_has_both_languages_and_a_unique_key() {
        let mut keys = std::collections::BTreeSet::new();
        for text in SystemText::ALL {
            assert!(keys.insert(text.key()), "{}", text.key());
            assert_eq!(SystemText::from_key(text.key()), Some(*text));
            for language in Language::ALL {
                assert_ne!(text.text(language), "");
            }
        }
        assert_eq!(Language::from_tag("pl-PL"), Language::Pl);
        assert_eq!(Language::from_tag("pl_PL"), Language::Pl);
        assert_eq!(Language::from_tag("de"), Language::En);
        assert_eq!(Language::from_tag(""), Language::En);
    }

    #[test]
    fn only_catalog_texts_are_localized() {
        let none = |_: &str, _: &str| None;
        let mut back = KeyFace::system(FaceStyle::Back, SystemText::Back);
        back.localize(Language::Pl, &none);
        assert_eq!(back.title, "Wstecz");
        back.localize(Language::En, &none);
        assert_eq!(back.title, "Back");

        // A title the user wrote stays, also next to a catalog detail.
        let mut custom =
            KeyFace::new(FaceStyle::Action, "Back").with_system_detail(SystemText::Unavailable);
        custom.localize(Language::Pl, &none);
        assert_eq!(custom.title, "Back");
        assert_eq!(custom.detail.as_deref(), Some("Niedostępne"));

        // A default action name, in either language; a renamed one stays.
        let mut next =
            KeyFace::new(FaceStyle::Action, "Next page").with_action_name("deck.next-page");
        next.localize(Language::Pl, &none);
        assert_eq!(next.title, "Następna strona");
        next.localize(Language::En, &none);
        assert_eq!(next.title, "Next page");
        let mut renamed =
            KeyFace::new(FaceStyle::Action, "Dalej").with_action_name("deck.next-page");
        renamed.localize(Language::Pl, &none);
        assert_eq!(renamed.title, "Dalej");

        // A plugin's names come from the caller.
        let plugin = |kind: &str, title: &str| {
            (kind == "acme.repo.open" && title == "Open repository").then(|| "Otwórz".to_owned())
        };
        let mut open =
            KeyFace::new(FaceStyle::Action, "Open repository").with_action_name("acme.repo.open");
        open.localize(Language::Pl, &plugin);
        assert_eq!(open.title, "Otwórz");

        // The marker is not part of the face's look.
        assert_eq!(
            KeyFace::system(FaceStyle::Back, SystemText::Back),
            KeyFace::new(FaceStyle::Back, "Back")
        );
    }

    #[test]
    fn a_state_label_replaces_a_catalog_title() {
        let face = KeyFace::new(FaceStyle::Action, "Next page")
            .with_action_name("deck.next-page")
            .with_state(&crate::ButtonState {
                label: Some("3 open".into()),
                ..crate::ButtonState::default()
            });
        assert!(!face.has_translated_text());
        let mut face = face;
        face.localize(Language::Pl, &|_, _| None);
        assert_eq!(face.title, "3 open");
    }

    #[test]
    fn tray_lines_are_formatted() {
        assert_eq!(
            format(
                SystemText::TrayProject,
                Language::Pl,
                &[("key", "DP"), ("name", "Deck")]
            ),
            "Projekt: DP – Deck"
        );
    }
}
