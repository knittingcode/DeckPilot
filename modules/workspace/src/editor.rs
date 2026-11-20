//! How editors and browsers are opened: the arguments known editors take
//! for a line, and the launch plan on each operating system. The plans
//! live in the core (`deckpilot_core::launch`), where the plugin host's
//! `resource.open` uses the same ones; pure, so every platform's plan is
//! tested everywhere.

#[cfg(test)]
use deckpilot_core::Os;
pub use deckpilot_core::launch::{EDITORS, Editor, MAX_POSITION, Position, editor_plan, url_plan};
use deckpilot_core::paths;
use deckpilot_module_system::support::{self, Failure};

/// A page to open in a browser: the System module's URL rules, and only
/// `http` or `https`.
pub fn browser_url(text: &str) -> Result<String, Failure> {
    let url = paths::web_url(text)?;
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(url)
    } else {
        Err(support::fail(
            deckpilot_module_system::codes::INVALID_URL,
            format!("“{}” must be an http or https address", text.trim()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn at(line: u32, column: Option<u32>) -> Option<Position> {
        Position::new(Some(line.into()), column.map(Into::into)).unwrap()
    }

    #[test]
    fn known_editors_get_their_line_arguments() {
        let file = PathBuf::from("/src/shop/main.rs");
        let args = |editor: Editor, column| editor.arguments(&file, Position { line: 12, column });
        assert_eq!(
            args(Editor::VsCode, Some(3)).unwrap(),
            ["--goto", "/src/shop/main.rs:12:3"]
        );
        assert_eq!(
            args(Editor::Cursor, None).unwrap(),
            ["--goto", "/src/shop/main.rs:12"]
        );
        assert_eq!(
            args(Editor::JetBrains, Some(3)).unwrap(),
            ["--line", "12", "--column", "3", "/src/shop/main.rs"]
        );
        assert_eq!(
            args(Editor::JetBrains, None).unwrap(),
            ["--line", "12", "/src/shop/main.rs"]
        );
        assert_eq!(args(Editor::Zed, None).unwrap(), ["/src/shop/main.rs:12"]);
        assert_eq!(
            args(Editor::Sublime, Some(1)).unwrap(),
            ["/src/shop/main.rs:12:1"]
        );
        assert_eq!(args(Editor::Other, None), None);
        assert_eq!(Editor::parse("zed"), Editor::Zed);
        assert_eq!(Editor::parse("notepad"), Editor::Other);
    }

    #[test]
    fn positions_are_checked() {
        assert_eq!(Position::new(None, None).unwrap(), None);
        assert_eq!(Position::new(Some(5), Some(2)).unwrap(), at(5, Some(2)));
        for (line, column) in [
            (Some(0), None),
            (Some(-1), None),
            (Some(MAX_POSITION + 1), None),
            (Some(1), Some(0)),
            (None, Some(3)),
            (Some(i64::MAX), None),
        ] {
            assert_eq!(
                Position::new(line, column).unwrap_err().code.as_deref(),
                Some(crate::codes::INVALID_POSITION),
                "{line:?} {column:?}"
            );
        }
    }

    #[test]
    fn editor_plans_on_every_system() {
        let file = PathBuf::from("/src/shop/main.rs");
        // No line: open with the application.
        let (mac, located) =
            editor_plan(Os::MacOs, Editor::VsCode, "Visual Studio Code", &file, None).unwrap();
        assert!(!located);
        assert_eq!(mac.program, "open");
        assert_eq!(mac.args, ["-a", "Visual Studio Code", "/src/shop/main.rs"]);
        let (linux, _) = editor_plan(Os::Linux, Editor::Zed, "/usr/bin/zed", &file, None).unwrap();
        assert_eq!(linux.program, "/usr/bin/zed");
        assert_eq!(linux.args, ["/src/shop/main.rs"]);

        // A line through an application bundle: `open -a … --args`.
        let (mac, located) = editor_plan(
            Os::MacOs,
            Editor::VsCode,
            "Visual Studio Code",
            &file,
            at(7, None),
        )
        .unwrap();
        assert!(located);
        assert_eq!(
            mac.args,
            [
                "-a",
                "Visual Studio Code",
                "--args",
                "--goto",
                "/src/shop/main.rs:7"
            ]
        );
        // A command-line tool is started directly, on macOS too.
        for os in [Os::MacOs, Os::Linux, Os::Windows] {
            let (plan, located) = editor_plan(
                os,
                Editor::JetBrains,
                "/usr/local/bin/idea",
                &file,
                at(7, Some(2)),
            )
            .unwrap();
            assert!(located, "{os:?}");
            assert_eq!(plan.program, "/usr/local/bin/idea");
            assert_eq!(
                plan.args,
                ["--line", "7", "--column", "2", "/src/shop/main.rs"]
            );
            assert!(!plan.wait);
        }
        let (windows, _) = editor_plan(
            Os::Windows,
            Editor::Sublime,
            r"C:\Program Files\Sublime Text\subl.exe",
            &file,
            at(3, None),
        )
        .unwrap();
        assert_eq!(windows.program, r"C:\Program Files\Sublime Text\subl.exe");
        assert_eq!(windows.args, ["/src/shop/main.rs:3"]);
        // A Linux .desktop application gets its arguments through gtk-launch.
        let (desktop, _) = editor_plan(
            Os::Linux,
            Editor::VsCode,
            "code.desktop",
            &file,
            at(3, None),
        )
        .unwrap();
        assert_eq!(desktop.program, "gtk-launch");
        assert_eq!(desktop.args, ["code", "--goto", "/src/shop/main.rs:3"]);

        // Unknown editors open the file without a line.
        let (plan, located) =
            editor_plan(Os::MacOs, Editor::Other, "TextEdit", &file, at(3, None)).unwrap();
        assert!(!located);
        assert_eq!(plan.args, ["-a", "TextEdit", "/src/shop/main.rs"]);
        assert!(editor_plan(Os::Other, Editor::Zed, "zed", &file, at(1, None)).is_err());
    }

    #[test]
    fn only_web_pages_open_in_a_browser() {
        assert_eq!(
            browser_url(" https://example.com/a ").unwrap(),
            "https://example.com/a"
        );
        assert!(browser_url("http://localhost:3000/").is_ok());
        for bad in [
            "mailto:team@example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "vscode://open",
            "example.com",
        ] {
            assert_eq!(
                browser_url(bad).unwrap_err().code.as_deref(),
                Some(deckpilot_module_system::codes::INVALID_URL),
                "{bad}"
            );
        }
    }

    #[test]
    fn url_plans_on_every_system() {
        let url = "https://example.com/";
        assert_eq!(
            url_plan(Os::MacOs, url, None).unwrap().args,
            ["https://example.com/"]
        );
        assert_eq!(url_plan(Os::Linux, url, None).unwrap().program, "xdg-open");
        assert_eq!(
            url_plan(Os::Windows, url, None).unwrap().args,
            ["url.dll,FileProtocolHandler", "https://example.com/"]
        );
        let firefox = url_plan(Os::MacOs, url, Some("Firefox")).unwrap();
        assert_eq!(firefox.args, ["-a", "Firefox", "https://example.com/"]);
        let linux = url_plan(Os::Linux, url, Some("/usr/bin/firefox")).unwrap();
        assert_eq!(linux.program, "/usr/bin/firefox");
        assert_eq!(linux.args, ["https://example.com/"]);
        let windows = url_plan(Os::Windows, url, Some(r"C:\Firefox\firefox.exe")).unwrap();
        assert_eq!(windows.program, r"C:\Firefox\firefox.exe");
        assert_eq!(windows.args, ["https://example.com/"]);
    }
}
