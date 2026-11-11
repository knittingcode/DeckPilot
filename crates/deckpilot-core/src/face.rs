//! Device-independent description of what a control should display.
//!
//! The runtime decides *what* a key shows ([`KeyFace`]); a renderer in the
//! agent turns it into pixels for the device's image format. Faces are
//! comparable, so unchanged keys can be skipped when refreshing a device.

use serde::{Deserialize, Serialize};

use crate::{ButtonState, Indicator, Project, StateSnapshot};

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
        }
    }

    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
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
        if let Some(label) = &state.label {
            self.title.clone_from(label);
        }
        if let Some(detail) = &state.detail {
            self.detail = Some(detail.clone());
        }
        self.indicator = state.indicator.or(self.indicator);
        self.progress = state.progress.map(|p| p.min(100)).or(self.progress);
        self.toggle = state.toggle.or(self.toggle);
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
                    face.detail = Some("Error".into());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectKey;

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
