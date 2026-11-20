//! Device-independent description of what a control should display.
//!
//! The runtime decides *what* a key shows ([`KeyFace`]); a renderer in the
//! agent turns it into pixels for the device's image format. Faces are
//! comparable, so unchanged keys can be skipped when refreshing a device.

use serde::{Deserialize, Serialize};

use crate::{Accent, ButtonState, Glyph, Indicator, KeyAppearance, Project, StateSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FaceStyle {
    /// Nothing bound; the key is blank.
    Empty,
    Action,
    Workflow,
    Folder,
    Back,
    /// Informational text such as "No layout".
    Notice,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyFace {
    pub style: FaceStyle,
    pub title: String,
    /// Secondary line, e.g. the active project of a project switcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Marks a key whose target is currently active.
    #[serde(default)]
    pub highlighted: bool,
    /// Live status mark from the key's [`ButtonState`] (or the agent, e.g.
    /// a running workflow).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indicator: Option<Indicator>,
    /// 0–100, drawn as a bar along the bottom edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    /// On/off state of a toggle-like action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toggle: Option<bool>,
    /// The binding's colours, title placement and icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<KeyAppearance>,
    /// The marks of the key's [`ButtonState`]: a background from the
    /// renderer's palette (the binding's own background wins), a shape
    /// above the title (the binding's own icon wins), a short label in the
    /// top left corner and a small chart of values 0–100 behind the texts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<Accent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<Glyph>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<u8>>,
    /// The key's icon by name from the UI icon set (the action's icon, else
    /// its type's; a folder, Back or workflow icon for those keys). The
    /// renderer draws it as the key's main mark unless the binding has its
    /// own icon or the state a glyph; a name it does not know is drawn as
    /// its fallback icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// Which of the texts DeckPilot produced itself, so the agent draws
    /// them in the effective language ([`crate::i18n`]). Not part of the
    /// look: ignored by comparisons and never serialized.
    #[serde(skip)]
    pub text: crate::i18n::FaceText,
}

impl KeyFace {
    pub fn empty() -> Self {
        Self {
            style: FaceStyle::Empty,
            title: String::new(),
            detail: None,
            highlighted: false,
            indicator: None,
            progress: None,
            toggle: None,
            appearance: None,
            accent: None,
            glyph: None,
            badge: None,
            series: None,
            symbol: None,
            text: crate::i18n::FaceText::default(),
        }
    }

    pub fn new(style: FaceStyle, title: impl Into<String>) -> Self {
        Self {
            style,
            title: title.into(),
            detail: None,
            highlighted: false,
            indicator: None,
            progress: None,
            toggle: None,
            appearance: None,
            accent: None,
            glyph: None,
            badge: None,
            series: None,
            symbol: None,
            text: crate::i18n::FaceText::default(),
        }
    }

    /// The face with `symbol` as its icon name (see [`KeyFace::symbol`]).
    #[must_use]
    pub fn with_symbol(mut self, symbol: Option<impl Into<String>>) -> Self {
        self.symbol = symbol.map(Into::into);
        self
    }

    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self.text.detail = None;
        self
    }

    #[must_use]
    pub fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }

    #[must_use]
    pub fn with_indicator(mut self, indicator: Indicator) -> Self {
        self.indicator = Some(indicator);
        self
    }

    #[must_use]
    pub fn with_progress(mut self, percent: Option<u8>) -> Self {
        self.progress = percent.map(|p| p.min(100));
        self
    }

    /// The face with a binding's appearance (`None` and default
    /// appearances leave it as it is).
    #[must_use]
    pub fn with_appearance(mut self, appearance: Option<&KeyAppearance>) -> Self {
        self.appearance = appearance.filter(|a| !a.is_default()).cloned();
        self
    }

    /// The face with an action type's title suggestions filled in where
    /// the binding's look sets nothing ([`KeyAppearance::with_suggestions`]).
    #[must_use]
    pub fn with_title_suggestions(mut self, title: Option<&crate::ActionTitle>) -> Self {
        if let Some(title) = title.filter(|title| title.position.is_some()) {
            self.appearance = self
                .appearance
                .take()
                .unwrap_or_default()
                .with_suggestions(title)
                .normalized();
        }
        self
    }

    /// The face with its title marked as the user's (see
    /// [`crate::i18n::FaceText::user_title`]) or not.
    #[must_use]
    pub fn with_user_title(mut self, user: bool) -> Self {
        self.text.user_title = user;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.style == FaceStyle::Empty
    }

    /// The face with a module-provided state applied: its label and detail
    /// replace the title and secondary line, and its indicator, progress
    /// and toggle are added. Empty and notice faces stay as they are.
    #[must_use]
    pub fn with_state(mut self, state: &ButtonState) -> Self {
        if matches!(self.style, FaceStyle::Empty | FaceStyle::Notice) {
            return self;
        }
        // A plugin's dynamic title, unless the user titled the key.
        if let Some(label) = &state.label
            && !self.text.user_title
        {
            self.title.clone_from(label);
            self.text.title = None;
        }
        if let Some(detail) = &state.detail {
            self.detail = Some(detail.clone());
            self.text.detail = None;
        }
        self.indicator = state.indicator.or(self.indicator);
        self.progress = state.progress.map(|p| p.min(100)).or(self.progress);
        self.toggle = state.toggle.or(self.toggle);
        self.accent = state.accent.or(self.accent);
        self.glyph = state.glyph.or(self.glyph);
        if let Some(badge) = &state.badge {
            self.badge = Some(badge.clone());
        }
        if let Some(series) = &state.series {
            self.series = Some(series.clone());
        }
        self
    }

    /// The face for what the agent knows about the key's state: a loading
    /// mark while the first state is pending, the state once it arrived, an
    /// error mark (and `Error` as the secondary line, unless the face has
    /// one) when it could not be determined.
    #[must_use]
    pub fn with_snapshot(self, snapshot: &StateSnapshot) -> Self {
        if matches!(self.style, FaceStyle::Empty | FaceStyle::Notice) {
            return self;
        }
        match snapshot {
            StateSnapshot::Loading => self.with_indicator(Indicator::Loading),
            StateSnapshot::Ready { state } => self.with_state(state),
            StateSnapshot::Failed { .. } => {
                let mut face = self.with_indicator(Indicator::Error);
                if face.detail.is_none() {
                    face = face.with_system_detail(crate::i18n::SystemText::Error);
                }
                face
            }
        }
    }
}

/// Expands `{profile}`, `{project.key}` and `{project.name}` in a key label.
/// Project placeholders show `-` while no project is active.
pub fn render_label(template: &str, profile: &str, project: Option<&Project>) -> String {
    let (key, name) = project.map_or(("-", "-"), |p| (p.key.as_str(), p.name.as_str()));
    template
        .replace("{profile}", profile)
        .replace("{project.key}", key)
        .replace("{project.name}", name)
}

/// Where a device is among the top-level pages of its layout, for the
/// `{page.…}` placeholders of key labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageLabel<'a> {
    /// The position of the top-level page, from 1.
    pub number: usize,
    /// How many top-level pages the layout has.
    pub count: usize,
    /// The name of the page shown (a folder's own name inside a folder).
    pub name: &'a str,
}

/// Expands `{page.number}`, `{page.count}` and `{page.name}` in the text
/// of a key face.
pub fn render_page_label(text: &str, page: PageLabel<'_>) -> String {
    if !text.contains("{page.") {
        return text.to_owned();
    }
    text.replace("{page.number}", &page.number.to_string())
        .replace("{page.count}", &page.count.to_string())
        .replace("{page.name}", page.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectKey;

    #[test]
    fn labels_show_the_page() {
        let page = PageLabel {
            number: 2,
            count: 5,
            name: "Media",
        };
        assert_eq!(render_page_label("{page.number}/{page.count}", page), "2/5");
        assert_eq!(render_page_label("On {page.name}", page), "On Media");
        assert_eq!(render_page_label("Plain {page}", page), "Plain {page}");
    }

    #[test]
    fn labels_follow_the_active_project() {
        let shop = Project::new(ProjectKey::new("SHOP").unwrap(), "Online shop");
        assert_eq!(
            render_label("Git {project.key}", "Work", Some(&shop)),
            "Git SHOP"
        );
        assert_eq!(
            render_label("{project.name}", "Work", Some(&shop)),
            "Online shop"
        );
        assert_eq!(
            render_label("{profile}: {project.key}", "Work", None),
            "Work: -"
        );
        assert_eq!(render_label("Plain", "Work", None), "Plain");
    }

    #[test]
    fn states_decorate_faces() {
        let face = KeyFace::new(FaceStyle::Action, "Repo").with_detail("SHOP");
        let ready = face.clone().with_snapshot(&StateSnapshot::Ready {
            state: ButtonState::new()
                .with_label("main")
                .with_indicator(Indicator::Warning)
                .with_progress(150)
                .with_toggle(true),
        });
        assert_eq!(ready.title, "main");
        assert_eq!(ready.detail.as_deref(), Some("SHOP"));
        assert_eq!(ready.indicator, Some(Indicator::Warning));
        assert_eq!(ready.progress, Some(100));
        assert_eq!(ready.toggle, Some(true));

        // The marks travel with the state; a failed or pending state has
        // none, so the key falls back to how the user designed it.
        let marked = face.clone().with_snapshot(&StateSnapshot::Ready {
            state: ButtonState::new()
                .with_accent(Accent::Green)
                .with_glyph(Glyph::Play)
                .with_badge("2")
                .with_series([1, 2, 3]),
        });
        assert_eq!(marked.title, "Repo");
        assert_eq!(marked.accent, Some(Accent::Green));
        assert_eq!(marked.glyph, Some(Glyph::Play));
        assert_eq!(marked.badge.as_deref(), Some("2"));
        assert_eq!(marked.series.as_deref(), Some(&[1, 2, 3][..]));
        for snapshot in [
            StateSnapshot::Loading,
            StateSnapshot::Failed {
                error: crate::StateError::new("x", "broken"),
            },
        ] {
            let plain = face.clone().with_snapshot(&snapshot);
            assert_eq!(
                (plain.accent, plain.glyph, plain.badge, plain.series),
                (None, None, None, None)
            );
        }
        let loading = face.clone().with_snapshot(&StateSnapshot::Loading);
        assert_eq!(loading.indicator, Some(Indicator::Loading));
        assert_eq!(loading.title, "Repo");

        let failed =
            KeyFace::new(FaceStyle::Action, "Repo").with_snapshot(&StateSnapshot::Failed {
                error: crate::StateError::new("x", "broken"),
            });
        assert_eq!(failed.indicator, Some(Indicator::Error));
        assert_eq!(failed.detail.as_deref(), Some("Error"));

        let notice =
            KeyFace::new(FaceStyle::Notice, "Locked").with_snapshot(&StateSnapshot::Loading);
        assert_eq!(notice.indicator, None);
    }

    #[test]
    fn faces_without_state_serialize_as_before() {
        let face = KeyFace::new(FaceStyle::Action, "A");
        assert_eq!(
            serde_json::to_value(&face).unwrap(),
            serde_json::json!({"style": "action", "title": "A", "highlighted": false})
        );
        let old: KeyFace =
            serde_json::from_value(serde_json::json!({"style": "action", "title": "A"})).unwrap();
        assert_eq!(old, face);
    }
}
