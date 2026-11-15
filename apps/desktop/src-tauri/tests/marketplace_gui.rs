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
        if cfg!(windows) {
            // A WebView2 profile of this run. msedgedriver finds the
            // webview's DevToolsActivePort in this folder (the app's own,
            // under %LOCALAPPDATA%, is not where it looks); the variable
            // takes precedence over the folder the app asks for.
            command.env("WEBVIEW2_USER_DATA_FOLDER", fx.root.path().join("webview2"));
        }
        if let Some(native) = &self.native_driver {
            command.arg("--native-driver").arg(native);
        }
        self.server = Some(command.spawn().expect("tauri-driver starts"));
        webdriver::wait_for_server(self.port, Duration::from_secs(60));
    }

    /// Starts the app and unlocks it in its unlock window.
    fn launch(&mut self, fx: &Fixture) -> Vec<Via> {
        self.starts += 1;
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
        self.wd()
            .wait(&button("Settings"), LONG)
            .expect("the main window");
        via
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

    /// Settings › Extensions › Plugins, unless it is open already.
    fn marketplace(&mut self) {
        if self.visible(&tab("Discover")) || self.visible(&button("Back to plugins")) {
            if self.visible(&button("Back to plugins")) {
                self.click(&button("Back to plugins"));
            }
            return;
        }
        if !self.visible(&button("Plugins")) {
            self.click(&button("Settings"));
        }
        self.click(&button("Plugins"));
        self.wait(&tab("Discover"));
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
        self.start_server(fx);
        let mut via = self.launch(fx);
        // The app started the agent (with the arguments and environment it
        // passes on), not the test.
        let status = self.ok("agent_status", json!({}));
        assert_eq!(status["running"], true, "{status}");
        via.push(Via::Ui);
        via
    }

    fn restart(&mut self, fx: &mut Fixture) -> Vec<Via> {
        // Settings › Background Agent › Stop agent.
        if !self.visible(&button("Background Agent")) {
            self.click(&button("Settings"));
        }
        self.click(&button("Background Agent"));
        self.click(&button("Stop agent"));
        fx.wait_for_agent(false);
        self.screenshot("12-agent-stopped");
        // Close the app and start it again: it starts the agent again,
        // which is locked, so the unlock window opens.
        if let Some(wd) = self.wd.take() {
            wd.quit().expect("the app closes");
        }
        let mut via = vec![Via::Ui];
        via.extend(self.launch(fx));
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
        self.marketplace();
        self.wait("//*[@role='note' and @aria-label='Experimental']");
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

    fn show_configuration(&mut self) -> Option<Vec<Via>> {
        self.installed_tab();
        self.click(&button("Configure"));
        let dialog = self.text_of(DIALOG);
        assert!(dialog.contains("Default city"), "{dialog}");
        assert!(dialog.contains("API token"), "{dialog}");
        self.screenshot("07-configure");
        // The dialog's close button (`sr-only` text "Close").
        let close = format!("{DIALOG}//button[normalize-space(.)='Close']");
        if self.wd().click(&close, Duration::from_secs(5)).is_err() {
            let _ = self.wd().execute(
                "document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))",
                &[],
            );
        }
        self.dialog_closed();
        Some(vec![Via::Ui])
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
