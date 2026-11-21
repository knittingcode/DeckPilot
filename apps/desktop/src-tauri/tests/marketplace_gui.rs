//! The marketplace scenario (`support/marketplace.rs`) through the built
//! app's UI: `WebDriver` (`tauri-driver` in front of `WebKitWebDriver` on
//! Linux or msedgedriver on Windows) drives the real webview; the app
//! starts the real agent itself (`DECKPILOT_AGENT_BIN`), which runs the
//! sample plugin in the real sandboxed host. macOS has no `WebDriver` for
//! `WKWebView`: there the scenario runs at the command layer only
//! (`src/marketplace_e2e.rs`).
//!
//! Steps the UI cannot do reliably go through the app's IPC instead: the
//! test calls the same command the frontend would, with
//! `window.__TAURI_INTERNALS__.invoke` inside the app's webview (so through
//! Tauri's real IPC and the app's process), and the step report says so.
//!
//! Ignored by default; `docs/marketplace/e2e.md` says how to run it:
//! `DECKPILOT_E2E_APP` is the app built with `VITE_DECKPILOT_PLUGINS=1
//! tauri build --debug --no-bundle`, `DECKPILOT_E2E_TAURI_DRIVER` the
//! `tauri-driver` executable (default: on `PATH`), and on Windows
//! `DECKPILOT_E2E_NATIVE_DRIVER` the msedgedriver matching `WebView2`.

#[path = "support/marketplace.rs"]
mod marketplace;
#[path = "support/webdriver.rs"]
mod webdriver;

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

use marketplace::{Driver, Fixture, HELLO, NAME, PASSWORD, Via, wait_until};
use webdriver::{WAIT, WebDriver};

const LONG: Duration = Duration::from_secs(90);
/// Whether the app's unlock window cannot be used through `WebDriver`
/// (Windows, see `Gui::launch_unlocked`).
const PROMPT_UNREACHABLE: bool = cfg!(windows);

/// An `XPath` string literal for `text` (no quotes of both kinds in it).
fn lit(text: &str) -> String {
    if text.contains('\'') {
        format!("\"{text}\"")
    } else {
        format!("'{text}'")
    }
}

fn button(text: &str) -> String {
    format!("//button[normalize-space(.)={}]", lit(text))
}

fn labelled(label: &str) -> String {
    format!("//button[@aria-label={}]", lit(label))
}

fn tab(name: &str) -> String {
    format!(
        "//button[@role='tab' and starts-with(normalize-space(.),{})]",
        lit(name)
    )
}

const DIALOG: &str = "//*[@role='dialog' or @role='alertdialog']";

/// The toolbar's Settings button (its label mentions an available update).
const SETTINGS: &str =
    "//header[@aria-label='Toolbar']//button[starts-with(@aria-label,'Settings')]";
/// The toolbar's Marketplace button.
const MARKETPLACE: &str = "//header[@aria-label='Toolbar']//button[@aria-label='Marketplace']";
/// The device editor, the main window's default view: the device drawn
/// from its descriptor, the Action Library at the right.
const EDITOR: &str =
    "//*[@data-slot='device-editor']//*[@role='group' and contains(@aria-label,'controls')]";
/// The library beside the editor, or (narrow windows) its overlay toggle.
const LIBRARY: &str = "//aside[@aria-label='Action Library'] | //*[@data-slot='device-editor']//button[normalize-space(.)='Library' and @aria-expanded]";

/// The settings view of the main window: its sidebar of sections.
const SETTINGS_VIEW: &str = "//*[@data-slot='sidebar' and @aria-label='Settings']";

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Gui {
    app: PathBuf,
    tauri_driver: PathBuf,
    native_driver: Option<PathBuf>,
    port: u16,
    server: Option<Child>,
    wd: Option<WebDriver>,
    artifacts: PathBuf,
    shots: u32,
    /// Restarts of the app, for the screenshot names.
    starts: u32,
}

impl Gui {
    fn new(fx: &Fixture) -> Self {
        let app = std::env::var_os("DECKPILOT_E2E_APP")
            .map(PathBuf::from)
            .expect("DECKPILOT_E2E_APP is not set (docs/marketplace/e2e.md)");
        assert!(
            app.exists() || app.with_extension("exe").exists(),
            "{}",
            app.display()
        );
        Self {
            app: std::path::absolute(app).unwrap(),
            tauri_driver: std::env::var_os("DECKPILOT_E2E_TAURI_DRIVER")
                .map_or_else(|| PathBuf::from("tauri-driver"), PathBuf::from),
            native_driver: std::env::var_os("DECKPILOT_E2E_NATIVE_DRIVER").map(PathBuf::from),
            port: free_port(),
            server: None,
            wd: None,
            artifacts: fx.artifacts.clone(),
            shots: 0,
            starts: 0,
        }
    }

    fn wd(&self) -> &WebDriver {
        self.wd.as_ref().expect("a WebDriver session")
    }

    /// `tauri-driver`, with the environment the app (and so the agent it
    /// starts) runs with.
    fn start_server(&mut self, fx: &Fixture) {
        let native = free_port();
        let log = std::fs::File::create(self.artifacts.join("tauri-driver.log")).unwrap();
        let mut command = Command::new(&self.tauri_driver);
        command
            .args(["--port", &self.port.to_string()])
            .args(["--native-port", &native.to_string()])
            .envs(fx.agent_env())
            .env("DECKPILOT_AGENT_BIN", &fx.agent_bin)
            .env("DECKPILOT_TEST_AGENT_ARGS", Fixture::AGENT_ARGS)
            .env_remove("DECKPILOT_NO_AGENT_AUTOSTART")
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log);
        // On Windows msedgedriver passes the webview its debugging port and
        // profile through WEBVIEW2_* variables, which the debug app applies
        // under tauri-driver (`windows::for_webdriver`).
        if let Some(native) = &self.native_driver {
            command.arg("--native-driver").arg(native);
        }
        self.server = Some(command.spawn().expect("tauri-driver starts"));
        webdriver::wait_for_server(self.port, Duration::from_secs(60));
    }

    /// Starts the app and unlocks it in its unlock window.
    fn launch(&mut self, fx: &mut Fixture) -> Vec<Via> {
        self.starts += 1;
        if PROMPT_UNREACHABLE {
            return self.launch_unlocked(fx);
        }
        let wd = WebDriver::start(self.port, &self.app).expect("a WebDriver session");
        self.wd = Some(wd);
        let main = self.wd().current_window().unwrap();
        // The agent the app started is locked: the app opens its unlock
        // window. Type the password there.
        let typed = self.unlock_in_prompt();
        let via = if typed {
            vec![Via::Ui]
        } else {
            // No prompt window reachable through WebDriver: unlock with
            // the unlock window's command, from the main window.
            self.wd().switch_to(&main).unwrap();
            self.ok("unlock", json!({"password": PASSWORD}));
            eprintln!("[gui] the unlock window was not reachable; unlocked via app IPC");
            vec![Via::AppIpc]
        };
        self.wd().switch_to(&main).unwrap();
        wait_until(LONG, "the unlocked agent", || {
            fx.agent_unlocked().then_some(())
        });
        self.wd().wait(SETTINGS, LONG).expect("the main window");
        marketplace::wait_for_bundled_plugins(self);
        via
    }

    /// Windows: the app finds the agent running and unlocked, takes its
    /// key and opens no prompt window. Under msedgedriver a second `WebView2`
    /// window of the app never loaded (`about:blank`) and the app stopped
    /// answering IPC (CI run 43742773500), so the test starts the agent
    /// itself (with the arguments the app's agent gets) and unlocks it over
    /// its IPC, with the key of the password.
    fn launch_unlocked(&mut self, fx: &mut Fixture) -> Vec<Via> {
        if !fx.agent_running() {
            fx.spawn_agent();
        }
        fx.unlock_agent();
        let wd = WebDriver::start(self.port, &self.app).expect("a WebDriver session");
        self.wd = Some(wd);
        self.wd().wait(SETTINGS, LONG).expect("the main window");
        eprintln!("[gui] the agent was started and unlocked by the test (Windows)");
        marketplace::wait_for_bundled_plugins(self);
        vec![Via::Fixture, Via::AgentIpc]
    }

    /// Finds the password prompt among the app's windows and submits the
    /// password there.
    fn unlock_in_prompt(&mut self) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(45);
        while std::time::Instant::now() < deadline {
            for handle in self.wd().windows().unwrap_or_default() {
                if self.wd().switch_to(&handle).is_err() {
                    continue;
                }
                let id = self
                    .wd()
                    .execute("return document.documentElement.dataset.window || ''", &[])
                    .unwrap_or_default();
                let id = id.as_str().unwrap_or_default().to_owned();
                if !id.starts_with("security.") {
                    continue;
                }
                if self
                    .wd()
                    .type_into("//input[@id='password']", PASSWORD, Duration::from_secs(5))
                    .is_ok()
                    && self
                        .wd()
                        .click(
                            "//button[@type='submit' or normalize-space(.)='Unlock']",
                            Duration::from_secs(5),
                        )
                        .is_ok()
                {
                    eprintln!("[gui] unlocked in the `{id}` window");
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        false
    }

    fn click(&self, xpath: &str) {
        self.wd()
            .click(xpath, WAIT)
            .unwrap_or_else(|e| panic!("click {xpath}: {e}"));
    }

    fn wait(&self, xpath: &str) -> String {
        self.wd()
            .wait(xpath, WAIT)
            .unwrap_or_else(|e| panic!("wait {xpath}: {e}"))
    }

    fn text_of(&self, xpath: &str) -> String {
        let element = self.wait(xpath);
        self.wd().text(&element).unwrap()
    }

    fn visible(&self, xpath: &str) -> bool {
        self.wd()
            .find_all(xpath)
            .unwrap_or_default()
            .iter()
            .any(|e| self.wd().displayed(e).unwrap_or(false))
    }

    /// Checks every unchecked consent box of the open dialog; how many.
    fn consent_all(&self) -> usize {
        let unchecked = format!("{DIALOG}//button[@role='checkbox' and @aria-checked='false']");
        let mut checked = 0;
        while let Some(element) = self.wd().find(&unchecked).unwrap() {
            let _ = element;
            self.click(&unchecked);
            checked += 1;
            assert!(checked < 10, "the consent boxes do not stay checked");
        }
        checked
    }

    fn dialog_closed(&self) {
        wait_until(WAIT, "the dialog to close", || {
            (!self.visible(DIALOG)).then_some(())
        });
    }

    /// The Marketplace (toolbar › Marketplace), unless it is open already.
    fn marketplace(&mut self) {
        if self.visible(&tab("Discover")) || self.visible(&button("Back to plugins")) {
            if self.visible(&button("Back to plugins")) {
                self.click(&button("Back to plugins"));
            }
            return;
        }
        self.click(MARKETPLACE);
        self.wait(&tab("Discover"));
    }

    /// The main window's default view: the device editor with the Action
    /// Library at the right and the key inspector below the device.
    /// Without a layout for the device (a fresh configuration may have
    /// none) the editor offers to create one instead.
    fn editor(&mut self) {
        // The rail marks the current page (`aria-current`); the toolbar has
        // no page title (the editor's own heading is visually hidden, and
        // WebDriver does not report it as displayed).
        self.wait("//button[@aria-current='page'][.//span[normalize-space(.)='Device Editor']]");
        if self.wd().wait(EDITOR, Duration::from_secs(10)).is_ok() {
            self.wait(LIBRARY);
            // The key inspector's region (empty until a key is selected).
            self.wait("//*[@data-slot='device-editor-main']//*[@data-slot='inspector-region']");
        } else {
            eprintln!("[gui] no layout for the device yet: the editor shows its empty state");
        }
    }

    /// Opens the settings from the toolbar: a view of the main window, so
    /// no other window appears and the session stays where it is.
    fn settings(&mut self) {
        let windows = self.wd().windows().unwrap_or_default().len();
        self.click(SETTINGS);
        self.wait(SETTINGS_VIEW);
        self.wait("//header[@aria-label='Toolbar']//button[starts-with(@aria-label,'Settings') and @aria-pressed='true']");
        assert_eq!(
            self.wd().windows().unwrap_or_default().len(),
            windows,
            "the settings open no window of their own"
        );
    }

    fn installed_tab(&mut self) {
        self.marketplace();
        self.click(&tab("Installed"));
        self.wait("//ul[@aria-label='Installed plugins']");
    }

    /// Installed › Hello Weather › Permissions and actions, expanded.
    fn expanded(&mut self) {
        self.installed_tab();
        let toggle = "//button[normalize-space(.)='Permissions and actions']";
        let element = self.wait(toggle);
        if self
            .wd()
            .attribute(&element, "aria-expanded")
            .unwrap()
            .as_deref()
            != Some("true")
        {
            self.click(toggle);
        }
        self.wait(&format!(
            "//*[@aria-label={}]",
            lit(&format!("Actions of {NAME}"))
        ));
    }

    fn plugin_field(&mut self, field: &str) -> Value {
        let details = self.ok("get_plugin", json!({"plugin": HELLO}));
        details["summary"][field].clone()
    }

    fn wait_plugin(&mut self, field: &str, value: &str) {
        wait_until(LONG, &format!("{field} = {value}"), || {
            (self.plugin_field(field) == value).then_some(())
        });
    }
}

impl Driver for Gui {
    fn name(&self) -> &'static str {
        "gui"
    }

    fn invoke(&mut self, command: &str, args: Value) -> Result<Value, Value> {
        let answer = self
            .wd()
            .execute_async(
                "const done = arguments[arguments.length - 1]; \
                 window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1]).then( \
                   (value) => done({ok: value === undefined ? null : value}), \
                   (error) => done({err: error === undefined ? null : error}))",
                &[json!(command), args],
            )
            .unwrap_or_else(|e| panic!("invoke {command}: {e}"));
        match answer.get("err") {
            Some(error) => Err(error.clone()),
            None => Ok(answer.get("ok").cloned().unwrap_or(Value::Null)),
        }
    }

    fn start(&mut self, fx: &mut Fixture) -> Vec<Via> {
        if PROMPT_UNREACHABLE {
            // As if the user had confirmed this version before: the app
            // takes the agent's key without asking again (security policy
            // "after updates"), so no confirmation window opens either.
            fx.confirm_app_version(env!("CARGO_PKG_VERSION"));
        }
        self.start_server(fx);
        let mut via = self.launch(fx);
        let status = self.ok("agent_status", json!({}));
        assert_eq!(status["running"], true, "{status}");
        if !PROMPT_UNREACHABLE {
            // The app started the agent (with the arguments and environment
            // it passes on), not the test.
            via.push(Via::Ui);
        }
        via
    }

    fn restart(&mut self, fx: &mut Fixture) -> Vec<Via> {
        // Toolbar › Settings (a view of the main window, so it is driven
        // on every platform) › Background Agent › Stop agent.
        self.settings();
        self.wait(&button("Background Agent"));
        self.screenshot("11-settings");
        self.click(&button("Background Agent"));
        self.click(&button("Stop agent"));
        let stopped_via = Via::Ui;
        fx.wait_for_agent(false);
        self.screenshot("12-agent-stopped");
        // Close the app and start it again: it starts the agent again,
        // which is locked, so the unlock window opens.
        if let Some(wd) = self.wd.take() {
            wd.quit().expect("the app closes");
        }
        if PROMPT_UNREACHABLE {
            fx.stop_agent();
        }
        let mut via = vec![stopped_via];
        via.extend(self.launch(fx));
        // The device editor again, now with the layout the scenario made:
        // the device, the Action Library at the right, the inspector below.
        self.click(&button("Device Editor"));
        self.editor();
        self.screenshot("12-shell-editor");
        via
    }

    fn screenshot(&mut self, name: &str) {
        self.shots += 1;
        if let Some(wd) = &self.wd {
            let path = self
                .artifacts
                .join(format!("gui-{:02}-{name}.png", self.shots));
            if let Err(e) = wd.screenshot(&path) {
                eprintln!("[gui] screenshot {name} failed: {e}");
            }
        }
    }

    fn open_marketplace(&mut self) -> Vec<Via> {
        // The shell first: the device editor is what the app opens with.
        self.editor();
        self.screenshot("02-shell-editor");
        self.marketplace();
        self.wait("//*[@role='note' and @aria-label='Experimental']");
        self.screenshot("02-marketplace");
        vec![Via::Ui]
    }

    fn discover(&mut self) -> Vec<Via> {
        self.click(&tab("Discover"));
        let card = format!(
            "//ul[@aria-label='Plugins']//button[.//span[normalize-space(.)={}]]",
            lit(NAME)
        );
        if self.wd().wait(&card, Duration::from_secs(10)).is_err() {
            // Not verified yet on this computer: fetch the registry.
            self.click(&button("Refresh"));
        }
        let text = self.text_of(&card);
        assert!(text.contains("Sample plugin"), "{text}");
        assert!(text.to_lowercase().contains("acme"), "{text}");
        vec![Via::Ui]
    }

    fn inspect(&mut self) -> Vec<Via> {
        let card = format!(
            "//ul[@aria-label='Plugins']//button[.//span[normalize-space(.)={}]]",
            lit(NAME)
        );
        self.click(&card);
        assert_eq!(self.text_of("//h3[@id='plugin-detail-name']"), NAME);
        let permissions = self.text_of("//section[@aria-labelledby='plugin-detail-permissions']");
        assert!(permissions.contains("api.open-meteo.com"), "{permissions}");
        let versions = self.text_of("//ul[@aria-label='Versions']");
        assert!(versions.contains("1.0.0"), "{versions}");
        vec![Via::Ui]
    }

    fn install(&mut self, consent: &[&str]) -> Vec<Via> {
        self.click(&button("Install 1.0.0"));
        self.wait(DIALOG);
        self.screenshot("05-consent");
        assert_eq!(self.consent_all(), consent.len(), "consent boxes");
        self.click(&format!("{DIALOG}{}", button("Install")));
        self.dialog_closed();
        self.wait_plugin("version", "1.0.0");
        wait_until(WAIT, "Installed 1.0.0 in the details", || {
            let status = self.text_of("//section[@aria-label='Install status']");
            status.contains("Installed 1.0.0").then_some(())
        });
        vec![Via::Ui]
    }

    fn enable(&mut self, _consent: &[&str]) -> Vec<Via> {
        self.installed_tab();
        self.click(&format!(
            "//button[@role='switch' and @aria-label={}]",
            lit(&format!("Enable {NAME}"))
        ));
        self.wait(DIALOG);
        self.screenshot("06-enable-consent");
        self.consent_all();
        self.click(&format!("{DIALOG}{}", button("Enable")));
        self.dialog_closed();
        self.wait_plugin("state", "enabled");
        self.wait(&format!(
            "//button[@role='switch' and @aria-label={}]",
            lit(&format!("Disable {NAME}"))
        ));
        vec![Via::Ui]
    }

    fn configure(&mut self) -> Vec<Via> {
        self.installed_tab();
        self.click(&button("Configure"));
        self.wait(&format!("{DIALOG}//form[@aria-label='Settings']"));
        // The setting: a choice (a Radix select, its options in a portal).
        self.click(&format!(
            "{DIALOG}//div[@role='group' and .//label[normalize-space(.)='Default city']]//button[@role='combobox']"
        ));
        self.click("//*[@role='option' and normalize-space(.)='tokyo']");
        self.click(&format!("{DIALOG}{}", button("Save settings")));
        wait_until(LONG, "the saved setting", || {
            let settings = self.ok("get_plugin_settings", json!({"plugin": HELLO}));
            (settings["values"]["defaultCity"] == "tokyo").then_some(())
        });
        // The credential: typed into its password field, never shown.
        self.click(&labelled("Set API token"));
        let form = "//form[@aria-label='Set API token']";
        self.wd()
            .type_into(
                &format!("{form}//input[@type='password']"),
                marketplace::SECRET,
                WAIT,
            )
            .unwrap();
        self.click(&format!("{form}{}", button("Store")));
        wait_until(LONG, "the stored credential", || {
            let slots = self.ok("list_plugin_credentials", json!({"plugin": HELLO}));
            slots
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["key"] == "token" && s["set"] == true)
                .then_some(())
        });
        let dialog = self.text_of(DIALOG);
        assert!(
            !dialog.contains(marketplace::SECRET),
            "the dialog shows the secret"
        );
        self.screenshot("07-configured");
        // The dialog's close button (`sr-only` text "Close").
        let close = format!("{DIALOG}//button[normalize-space(.)='Close']");
        if self.wd().click(&close, Duration::from_secs(5)).is_err() {
            let _ = self.wd().execute(
                "document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))",
                &[],
            );
        }
        self.dialog_closed();
        vec![Via::Ui]
    }

    fn run_plugin_action(&mut self, action: &str, params: Value) -> (Vec<Via>, String) {
        self.expanded();
        let (name, labels): (&str, &[(&str, &str)]) = match action {
            "greet" => ("Greet", &[("name", "Name")]),
            _ => ("Current temperature", &[("city", "City")]),
        };
        let row = format!(
            "//li[@aria-labelledby={}]",
            lit(&format!("action-{HELLO}-{action}-name"))
        );
        for (key, label) in labels {
            let value = params[*key].as_str().unwrap();
            self.wd()
                .type_into(
                    &format!(
                        "{row}//label[span[normalize-space(.)={}]]//input",
                        lit(label)
                    ),
                    value,
                    WAIT,
                )
                .unwrap();
        }
        self.click(&labelled(&format!("Run {name}")));
        let result = format!(
            "//*[@role='status' and @aria-label={}]",
            lit(&format!("Result of {name}"))
        );
        let text = wait_until(LONG, "the action's result", || {
            let element = self.wd().find(&result).ok().flatten()?;
            let text = self.wd().text(&element).ok()?;
            (!text.trim().is_empty()).then_some(text)
        });
        assert!(text.starts_with("Succeeded"), "{text}");
        (vec![Via::Ui], text)
    }

    fn update(&mut self, consent: &[&str]) -> Vec<Via> {
        self.marketplace();
        self.click(&tab("Updates"));
        self.click(&button("Check for updates"));
        let item = format!(
            "//li[@aria-label={}]",
            lit(&format!("{NAME} 1.0.0 to 1.1.0"))
        );
        self.wait(&item);
        self.screenshot("14-update-available");
        self.click(&format!("{item}//button"));
        self.wait(DIALOG);
        self.screenshot("14-update-consent");
        assert_eq!(self.consent_all(), consent.len(), "consent boxes");
        self.click(&format!("{DIALOG}{}", button("Update")));
        self.dialog_closed();
        self.wait_plugin("version", "1.1.0");
        vec![Via::Ui]
    }

    fn rollback(&mut self) -> Vec<Via> {
        self.installed_tab();
        self.click(&button("Roll back to 1.0.0"));
        self.wait_plugin("version", "1.0.0");
        vec![Via::Ui]
    }

    fn revoke(&mut self, permission: &str) -> Vec<Via> {
        assert_eq!(permission, marketplace::NETWORK);
        self.expanded();
        self.click(&labelled(
            "Revoke: Send data to and receive data from https://api.open-meteo.com",
        ));
        wait_until(LONG, "the revoked permission", || {
            let details = self.ok("get_plugin", json!({"plugin": HELLO}));
            details["permissions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["key"] == permission && p["status"] == "revoked")
                .then_some(())
        });
        vec![Via::Ui]
    }

    fn denied_in_ui(&mut self) -> Option<Vec<Via>> {
        self.expanded();
        let run = labelled("Run Current temperature");
        let element = wait_until(WAIT, "the Run button to be disabled", || {
            let element = self.wd().find(&run).ok().flatten()?;
            (!self.wd().enabled(&element).unwrap_or(true)).then_some(element)
        });
        let _ = element;
        let row = self.text_of(&format!(
            "//li[@aria-labelledby={}]",
            lit(&format!("action-{HELLO}-current-temperature-name"))
        ));
        assert!(row.contains("not granted"), "{row}");
        Some(vec![Via::Ui])
    }

    fn disable(&mut self) -> Vec<Via> {
        self.installed_tab();
        self.click(&format!(
            "//button[@role='switch' and @aria-label={}]",
            lit(&format!("Disable {NAME}"))
        ));
        self.wait_plugin("state", "disabled");
        vec![Via::Ui]
    }

    fn uninstall(&mut self) -> Vec<Via> {
        self.installed_tab();
        self.click(&format!(
            "//ul[@aria-label='Installed plugins']{}",
            button("Uninstall")
        ));
        self.wait(DIALOG);
        self.screenshot("20-uninstall-confirm");
        // The default retention: nothing kept.
        for keep in ["Keep its settings", "Keep its credentials"] {
            let element = self.wait(&format!(
                "{DIALOG}//button[@role='checkbox' and @aria-label={}]",
                lit(keep)
            ));
            assert_eq!(
                self.wd()
                    .attribute(&element, "aria-checked")
                    .unwrap()
                    .as_deref(),
                Some("false"),
                "{keep}"
            );
        }
        self.click(&format!("{DIALOG}{}", button("Uninstall")));
        self.dialog_closed();
        wait_until(LONG, "the plugin to be gone", || {
            let plugins = self.ok("list_plugins", json!({}));
            plugins
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["id"] != HELLO)
                .then_some(())
        });
        vec![Via::Ui]
    }
}

impl Drop for Gui {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Some(wd) = &self.wd
        {
            // Every window: a screenshot and its DOM.
            let current = wd.current_window().ok();
            for (index, handle) in wd.windows().unwrap_or_default().iter().enumerate() {
                if wd.switch_to(handle).is_ok() {
                    let _ =
                        wd.screenshot(&self.artifacts.join(format!("failure-window-{index}.png")));
                    if let Ok(source) = wd.source() {
                        let _ = std::fs::write(
                            self.artifacts.join(format!("failure-window-{index}.html")),
                            source,
                        );
                    }
                }
            }
            if let Some(current) = current {
                let _ = wd.switch_to(&current);
            }
        }
        if let Some(wd) = self.wd.take() {
            let _ = wd.quit();
        }
        if let Some(mut server) = self.server.take() {
            let _ = server.kill();
            let _ = server.wait();
        }
    }
}

#[test]
#[ignore = "needs the built app, tauri-driver and the plugin artifacts (docs/marketplace/e2e.md)"]
fn the_marketplace_works_end_to_end_through_the_ui() {
    let mut fx = Fixture::new("marketplace-gui");
    let mut gui = Gui::new(&fx);
    marketplace::run(&mut gui, &mut fx);
    drop(gui);
}
