//! Device-independent description of what a control should display.
//!
//! The runtime decides *what* a key shows ([`KeyFace`]); a renderer in the
//! agent turns it into pixels for the device's image format. Faces are
//! comparable, so unchanged keys can be skipped when refreshing a device.

use serde::{Deserialize, Serialize};

use crate::Project;

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
}

impl KeyFace {
    pub fn empty() -> Self {
        Self {
            style: FaceStyle::Empty,
            title: String::new(),
            detail: None,
            highlighted: false,
        }
    }

    pub fn new(style: FaceStyle, title: impl Into<String>) -> Self {
        Self {
            style,
            title: title.into(),
            detail: None,
            highlighted: false,
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

    pub fn is_empty(&self) -> bool {
        self.style == FaceStyle::Empty
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
}
