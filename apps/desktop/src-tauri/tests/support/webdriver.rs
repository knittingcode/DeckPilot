//! A minimal W3C `WebDriver` client for `tauri-driver`: sessions, windows,
//! elements found by `XPath`, clicks, typing, scripts and screenshots.
//! Blocking, over the app's own HTTP client (reqwest) on a private
//! runtime; nothing beyond what the marketplace scenario needs.

#![allow(dead_code)]

use std::path::Path;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};

const ELEMENT: &str = "element-6066-11e4-a52e-4f735466cecf";
/// How long a wait for an element lasts by default.
pub const WAIT: Duration = Duration::from_secs(30);

/// A `WebDriver` error: the W3C error code and message.
#[derive(Debug, Clone)]
pub struct WdError {
    pub error: String,
    pub message: String,
}

impl std::fmt::Display for WdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.error, self.message)
    }
}

pub type WdResult<T> = Result<T, WdError>;

#[derive(Debug)]
pub struct WebDriver {
    http: reqwest::Client,
    base: String,
    session: String,
    runtime: tokio::runtime::Runtime,
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(180))
        .build()
        .unwrap()
}

async fn send(
    http: &reqwest::Client,
    method: reqwest::Method,
    url: String,
    body: Option<&Value>,
) -> WdResult<Value> {
    let mut request = http.request(method, &url);
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(serde_json::to_vec(body).unwrap());
    }
    let response = request.send().await.map_err(|e| WdError {
        error: "connection".into(),
        message: format!("{url}: {e}"),
    })?;
    let bytes = response.bytes().await.map_err(|e| WdError {
        error: "connection".into(),
        message: e.to_string(),
    })?;
    let answer: Value = serde_json::from_slice(&bytes).map_err(|e| WdError {
        error: "invalid answer".into(),
        message: format!("{e}: {}", String::from_utf8_lossy(&bytes)),
    })?;
    let value = answer.get("value").cloned().unwrap_or(Value::Null);
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(WdError {
            error: error.to_owned(),
            message: value["message"].as_str().unwrap_or_default().to_owned(),
        });
    }
    Ok(value)
}

/// Waits until `tauri-driver` answers on `port`.
pub fn wait_for_server(port: u16, timeout: Duration) {
    let runtime = runtime();
    let http = client();
    let deadline = Instant::now() + timeout;
    loop {
        let status = runtime.block_on(send(
            &http,
            reqwest::Method::GET,
            format!("http://127.0.0.1:{port}/status"),
            None,
        ));
        if status.is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "tauri-driver did not answer on port {port}: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

impl WebDriver {
    /// A session that starts `application` (`tauri:options`).
    pub fn start(port: u16, application: &Path) -> WdResult<Self> {
        let runtime = runtime();
        let http = client();
        let base = format!("http://127.0.0.1:{port}");
        let capabilities = json!({"capabilities": {"alwaysMatch": {
            "tauri:options": {"application": application}
        }}});
        let value = runtime.block_on(send(
            &http,
            reqwest::Method::POST,
            format!("{base}/session"),
            Some(&capabilities),
        ))?;
        let session = value["sessionId"].as_str().unwrap_or_default().to_owned();
        assert!(!session.is_empty(), "no session id: {value}");
        let driver = Self {
            http,
            base,
            session,
            runtime,
        };
        driver.post(
            "timeouts",
            &json!({"script": 120_000, "pageLoad": 120_000, "implicit": 0}),
        )?;
        Ok(driver)
    }

    fn url(&self, path: &str) -> String {
        format!("{}/session/{}/{path}", self.base, self.session)
    }

    fn post(&self, path: &str, body: &Value) -> WdResult<Value> {
        self.runtime.block_on(send(
            &self.http,
            reqwest::Method::POST,
            self.url(path),
            Some(body),
        ))
    }

    fn get(&self, path: &str) -> WdResult<Value> {
        self.runtime
            .block_on(send(&self.http, reqwest::Method::GET, self.url(path), None))
    }

    /// Ends the session (the app closes).
    pub fn quit(self) -> WdResult<()> {
        self.runtime
            .block_on(send(
                &self.http,
                reqwest::Method::DELETE,
                format!("{}/session/{}", self.base, self.session),
                None,
            ))
            .map(|_| ())
    }

    pub fn windows(&self) -> WdResult<Vec<String>> {
        Ok(self
            .get("window/handles")?
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|h| h.as_str().map(str::to_owned))
            .collect())
    }

    pub fn current_window(&self) -> WdResult<String> {
        Ok(self.get("window")?.as_str().unwrap_or_default().to_owned())
    }

    pub fn switch_to(&self, handle: &str) -> WdResult<()> {
        self.post("window", &json!({"handle": handle})).map(|_| ())
    }

    /// Runs `script` (a function body; `arguments` are `args`).
    pub fn execute(&self, script: &str, args: &[Value]) -> WdResult<Value> {
        self.post("execute/sync", &json!({"script": script, "args": args}))
    }

    /// Runs `script`, which calls `arguments[arguments.length - 1]` with
    /// its result.
    pub fn execute_async(&self, script: &str, args: &[Value]) -> WdResult<Value> {
        self.post("execute/async", &json!({"script": script, "args": args}))
    }

    pub fn find_all(&self, xpath: &str) -> WdResult<Vec<String>> {
        Ok(self
            .post("elements", &json!({"using": "xpath", "value": xpath}))?
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|e| e[ELEMENT].as_str().map(str::to_owned))
            .collect())
    }

    pub fn find(&self, xpath: &str) -> WdResult<Option<String>> {
        Ok(self.find_all(xpath)?.into_iter().next())
    }

    /// The first element matching `xpath` that is displayed, waiting up to
    /// `timeout`.
    pub fn wait(&self, xpath: &str, timeout: Duration) -> WdResult<String> {
        let deadline = Instant::now() + timeout;
        let mut last = None;
        loop {
            match self.find_all(xpath) {
                Ok(found) => {
                    for element in found {
                        if self.displayed(&element).unwrap_or(false) {
                            return Ok(element);
                        }
                    }
                }
                Err(e) => last = Some(e),
            }
            if Instant::now() >= deadline {
                return Err(WdError {
                    error: "timeout".into(),
                    message: format!(
                        "no displayed element for {xpath} within {timeout:?}{}",
                        last.map(|e| format!(" (last error: {e})"))
                            .unwrap_or_default()
                    ),
                });
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn displayed(&self, element: &str) -> WdResult<bool> {
        // `displayed` is not a W3C endpoint every driver has; ask the page.
        let shown = self.execute(
            "const e = arguments[0]; const r = e.getBoundingClientRect(); \
             const s = getComputedStyle(e); \
             return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';",
            &[json!({ELEMENT: element})],
        )?;
        Ok(shown.as_bool().unwrap_or(false))
    }

    pub fn enabled(&self, element: &str) -> WdResult<bool> {
        Ok(self
            .get(&format!("element/{element}/enabled"))?
            .as_bool()
            .unwrap_or(false))
    }

    pub fn attribute(&self, element: &str, name: &str) -> WdResult<Option<String>> {
        Ok(self
            .get(&format!("element/{element}/attribute/{name}"))?
            .as_str()
            .map(str::to_owned))
    }

    pub fn text(&self, element: &str) -> WdResult<String> {
        Ok(self
            .get(&format!("element/{element}/text"))?
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    /// Clicks the element: scrolled into view first, retried while
    /// something (an animation, an overlay going away) is in the way.
    pub fn click(&self, xpath: &str, timeout: Duration) -> WdResult<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let element = self.wait(xpath, deadline.saturating_duration_since(Instant::now()))?;
            let _ = self.execute(
                "arguments[0].scrollIntoView({block: 'center', inline: 'center'});",
                &[json!({ELEMENT: element})],
            );
            match self.post(&format!("element/{element}/click"), &json!({})) {
                Ok(_) => return Ok(()),
                Err(e) if Instant::now() < deadline => {
                    eprintln!("[webdriver] click on {xpath} failed ({e}); retrying");
                    std::thread::sleep(Duration::from_millis(300));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Replaces the text of an input.
    pub fn type_into(&self, xpath: &str, text: &str, timeout: Duration) -> WdResult<()> {
        let element = self.wait(xpath, timeout)?;
        self.post(&format!("element/{element}/clear"), &json!({}))?;
        self.post(&format!("element/{element}/value"), &json!({"text": text}))
            .map(|_| ())
    }

    pub fn screenshot(&self, path: &Path) -> WdResult<()> {
        let data = self.get("screenshot")?;
        let png = base64::engine::general_purpose::STANDARD
            .decode(data.as_str().unwrap_or_default())
            .map_err(|e| WdError {
                error: "invalid screenshot".into(),
                message: e.to_string(),
            })?;
        std::fs::write(path, png).map_err(|e| WdError {
            error: "io".into(),
            message: e.to_string(),
        })
    }

    pub fn source(&self) -> WdResult<String> {
        Ok(self.get("source")?.as_str().unwrap_or_default().to_owned())
    }
}
