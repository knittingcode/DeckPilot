//! The action types of the `http` module.

use std::sync::Arc;
use std::time::Duration;

use deckpilot_core::graph::{ActionEffect, NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, Capability,
    CredentialId, ExecutionContext, ParameterKind, ParameterSpec, ParameterValue, Profile,
};

use crate::client::{self, Auth};
use crate::{HttpSettings, codes, policy};

pub const REQUEST: &str = "http.request";
pub const HEALTH_CHECK: &str = "http.health-check";

pub const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
const SCHEMES: [&str; 4] = ["bearer", "basic", "header", "query"];
/// Longest request body, in characters.
pub const MAX_BODY_CHARS: usize = 64 * 1024;

type Failure = ActionResult;

fn fail(code: &str, message: impl Into<String>) -> Failure {
    ActionResult::error(code, message)
}

fn failed((code, message): client::Failure) -> Failure {
    fail(code, message)
}

/// # Panics
///
/// Never for the constants of this module.
pub fn id(value: &str) -> ActionTypeId {
    ActionTypeId::new(value).expect("the action type id is valid")
}

pub fn handlers(settings: &Arc<HttpSettings>) -> Vec<Box<dyn ActionHandler>> {
    vec![
        Box::new(HttpAction {
            definition: request_definition(),
            health: false,
            settings: settings.clone(),
        }),
        Box::new(HttpAction {
            definition: health_definition(),
            health: true,
            settings: settings.clone(),
        }),
    ]
}

fn parameter(key: &str, label: &str, kind: ParameterKind, description: &str) -> ParameterSpec {
    ParameterSpec {
        key: key.into(),
        label: label.into(),
        description: (!description.is_empty()).then(|| description.into()),
        kind,
        required: false,
        default: None,
    }
}

fn text(max_length: usize, multiline: bool) -> ParameterKind {
    ParameterKind::Text {
        multiline,
        max_length: Some(max_length),
    }
}

fn choice(options: &[&str]) -> ParameterKind {
    ParameterKind::Choice {
        options: options.iter().map(|o| (*o).to_owned()).collect(),
    }
}

fn required(mut spec: ParameterSpec, default: Option<ParameterValue>) -> ParameterSpec {
    spec.required = true;
    spec.default = default;
    spec
}

fn timeout(default: i64, max: i64) -> ParameterSpec {
    required(
        parameter(
            "timeoutSeconds",
            "Time limit (s)",
            ParameterKind::Integer {
                min: Some(1),
                max: Some(max),
            },
            "",
        ),
        Some(ParameterValue::Integer(default)),
    )
}

fn credential_parameters() -> Vec<ParameterSpec> {
    vec![
        parameter(
            "credential",
            "Credential",
            ParameterKind::Credential,
            "The name of a credential stored on this computer (Settings › HTTP). The secret \
             is never part of the workflow.",
        ),
        required(
            parameter(
                "authScheme",
                "Send it as",
                choice(&SCHEMES),
                "bearer: Authorization: Bearer; basic: Authorization: Basic with user:password; \
                 header or query: under the name below.",
            ),
            Some(ParameterValue::Choice("bearer".into())),
        ),
        parameter(
            "authName",
            "Header or parameter name",
            text(64, false),
            "For header (default X-API-Key) and query (default api_key).",
        ),
    ]
}

fn url_parameter() -> ParameterSpec {
    required(
        parameter(
            "url",
            "URL",
            text(4096, false),
            "An http or https address. Addresses on this computer or a private network must \
             be allowed in Settings › HTTP.",
        ),
        None,
    )
}

fn out(key: &str, label: &str, kind: ValueKind) -> OutputSpec {
    OutputSpec::new(key, label, kind)
}

#[allow(clippy::too_many_lines)] // One declaration of the request parameters.
fn request_definition() -> ActionType {
    let mut parameters = vec![
        required(
            parameter("method", "Method", choice(&METHODS), ""),
            Some(ParameterValue::Choice("GET".into())),
        ),
        url_parameter(),
        parameter(
            "query",
            "Query parameters",
            ParameterKind::Pairs {
                separator: "=".into(),
                name_label: Some("Parameter".into()),
                value_label: Some("Value".into()),
                max_items: Some(50),
                case_insensitive: false,
                no_secrets: true,
            },
            "Values are encoded. Secrets belong in a stored credential.",
        ),
        parameter(
            "headers",
            "Headers",
            ParameterKind::Pairs {
                separator: ":".into(),
                name_label: Some("Header".into()),
                value_label: Some("Value".into()),
                max_items: Some(50),
                case_insensitive: true,
                no_secrets: true,
            },
            "Secrets (Authorization, cookies, tokens) belong in a stored credential.",
        ),
        parameter(
            "body",
            "Body",
            ParameterKind::Code {
                language: "json".into(),
                max_length: Some(MAX_BODY_CHARS),
            },
            "",
        ),
        required(
            parameter(
                "bodyType",
                "Body type",
                choice(&["json", "text"]),
                "json checks that the body is JSON and sends it as application/json.",
            ),
            Some(ParameterValue::Choice("json".into())),
        ),
    ];
    parameters.extend(credential_parameters());
    parameters.extend([
        timeout(30, 300),
        required(
            parameter(
                "followRedirects",
                "Follow redirects",
                ParameterKind::Boolean,
                "Up to 5, each checked like the first request. Never from https to http; a \
                 credential is not sent to another server.",
            ),
            Some(ParameterValue::Boolean(true)),
        ),
        required(
            parameter(
                "failOnHttpError",
                "Fail on 4xx and 5xx",
                ParameterKind::Boolean,
                "When off, the status is only reported.",
            ),
            Some(ParameterValue::Boolean(true)),
        ),
        parameter(
            "extract",
            "Read a JSON value",
            text(256, false),
            "A JSON pointer such as /data/0/id; its value becomes the `value` output.",
        ),
    ]);
    ActionType {
        id: id(REQUEST),
        name: "HTTP request".into(),
        description: "Sends an HTTP request and reports the status, headers and body (JSON or \
                      text)."
            .into(),
        category: "HTTP".into(),
        parameters,
        requires_project: false,
        node: NodeSpec {
            icon: Some("globe".into()),
            outputs: vec![
                out("status", "Status", ValueKind::Integer),
                out("ok", "Successful (2xx)", ValueKind::Boolean),
                out("contentType", "Content type", ValueKind::Text),
                out("headers", "Headers", ValueKind::Text),
                out("body", "Body", ValueKind::Text).sensitive(),
                out("value", "Extracted value", ValueKind::Text)
                    .optional()
                    .sensitive(),
                out("size", "Body size (bytes)", ValueKind::Integer),
                out("truncated", "Body truncated", ValueKind::Boolean),
                out("url", "Final URL (without query)", ValueKind::Url),
                out("redirects", "Redirects", ValueKind::Integer),
                out("durationMs", "Duration (ms)", ValueKind::Integer),
            ],
            timeout_ms: 310_000,
            // A request may change data on the server (POST, DELETE, …).
            retry_safe: false,
            idempotent: false,
            cancellable: true,
            effect: ActionEffect::Modifies,
            ..NodeSpec::default()
        },
    }
}

fn health_definition() -> ActionType {
    let mut parameters = vec![
        url_parameter(),
        required(
            parameter(
                "expectedStatus",
                "Expected status",
                text(64, false),
                "e.g. 200, 2xx, 200-299 or 200,204.",
            ),
            Some(ParameterValue::Text("2xx".into())),
        ),
        parameter(
            "contains",
            "Body contains",
            text(256, false),
            "Healthy only if the body contains this text.",
        ),
        timeout(10, 120),
        required(
            parameter(
                "failWhenUnhealthy",
                "Fail when unhealthy",
                ParameterKind::Boolean,
                "When off, the node succeeds and `healthy` says how it went.",
            ),
            Some(ParameterValue::Boolean(true)),
        ),
    ];
    parameters.extend(credential_parameters());
    ActionType {
        id: id(HEALTH_CHECK),
        name: "Health check".into(),
        description: "Requests a URL with GET and checks its status and, optionally, its body."
            .into(),
        category: "HTTP".into(),
        parameters,
        requires_project: false,
        node: NodeSpec {
            icon: Some("activity".into()),
            outputs: vec![
                out("healthy", "Healthy", ValueKind::Boolean),
                out("status", "Status", ValueKind::Integer).optional(),
                out("reason", "Why not healthy", ValueKind::Text).optional(),
                out("durationMs", "Duration (ms)", ValueKind::Integer),
            ],
            timeout_ms: 130_000,
            retry_safe: true,
            idempotent: true,
            cancellable: true,
            effect: ActionEffect::ReadOnly,
            ..NodeSpec::default()
        },
    }
}

/// Status expectations: `200`, `2xx`, `200-299`, `200,204`.
pub fn status_matches(expected: &str, status: u16) -> Option<bool> {
    let mut any = false;
    for part in expected.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let matches =
            if let Some(class) = part.strip_suffix("xx").or_else(|| part.strip_suffix("XX")) {
                let class: u16 = class.parse().ok().filter(|c| (1..=5).contains(c))?;
                status / 100 == class
            } else if let Some((low, high)) = part.split_once('-') {
                let (low, high): (u16, u16) = (low.trim().parse().ok()?, high.trim().parse().ok()?);
                (low..=high).contains(&status)
            } else {
                part.parse::<u16>().ok()? == status
            };
        any |= matches;
    }
    Some(any)
}

#[derive(Debug)]
struct HttpAction {
    definition: ActionType,
    health: bool,
    settings: Arc<HttpSettings>,
}

fn optional<'a>(parameters: &'a ActionParameters, key: &str) -> Option<&'a str> {
    parameters
        .text(key)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

impl ActionHandler for HttpAction {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let result = if self.health {
            self.health_check(parameters, context)
        } else {
            self.request(parameters, context)
        };
        result.unwrap_or_else(|failure| failure)
    }

    /// Problems found when the action is saved, so a secret never reaches
    /// the profile.
    fn check(&self, parameters: &ActionParameters, _: &Profile) -> Vec<String> {
        let mut problems = Vec::new();
        if let Some(url) = optional(parameters, "url")
            && !url.contains("{{")
            && let Err((_, message)) = policy::request_url(url)
        {
            problems.push(message);
        }
        if let Some(headers) = optional(parameters, "headers")
            && let Err((_, message)) = policy::headers(headers)
        {
            problems.push(message);
        }
        if let Some(query) = optional(parameters, "query")
            && let Err((_, message)) = policy::query(query)
        {
            problems.push(message);
        }
        if let Some(credential) = optional(parameters, "credential")
            && CredentialId::new(credential).is_err()
        {
            problems.push(format!("“{credential}” is not a credential name"));
        }
        if let Some(expected) = optional(parameters, "expectedStatus")
            && status_matches(expected, 200).is_none()
        {
            problems.push(format!(
                "“{expected}” is not a status such as 200, 2xx or 200-299"
            ));
        }
        problems
    }
}

impl HttpAction {
    fn auth(
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<Option<Auth>, Failure> {
        let Some(name) = optional(parameters, "credential") else {
            return Ok(None);
        };
        let id = CredentialId::new(name).map_err(|_| {
            fail(
                "invalid-parameters",
                format!("“{name}” is not a credential name"),
            )
        })?;
        let secret = context
            .credential(&id)
            .map_err(|e| fail(e.code(), e.to_string()))?;
        let custom = optional(parameters, "authName");
        Ok(Some(
            match parameters.text("authScheme").unwrap_or("bearer") {
                "basic" => Auth::Basic(secret),
                "header" => {
                    let header = custom.unwrap_or("X-API-Key");
                    if policy::headers(&format!("{header}: x")).is_err()
                        && !policy::sensitive_header(header)
                    {
                        return Err(fail(
                            codes::INVALID_HEADER,
                            format!("{header} is not a header name"),
                        ));
                    }
                    Auth::Header(header.to_owned(), secret)
                }
                "query" => Auth::Query(custom.unwrap_or("api_key").to_owned(), secret),
                _ => Auth::Bearer(secret),
            },
        ))
    }

    fn timeout(parameters: &ActionParameters, max: i64) -> Duration {
        Duration::from_secs(
            parameters
                .integer("timeoutSeconds")
                .unwrap_or(30)
                .clamp(1, max)
                .unsigned_abs(),
        )
    }

    fn send(
        &self,
        request: client::Request,
        context: &ExecutionContext<'_>,
    ) -> Result<client::Response, Failure> {
        context
            .permit(Capability::Network)
            .map_err(ActionResult::from)?;
        context.report_progress(
            None,
            format!("{} {}", request.method, policy::shown_url(&request.url)),
        );
        client::send(request, &self.settings.network, context.cancellation()).map_err(failed)
    }

    fn request(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ActionResult, Failure> {
        let mut url =
            policy::request_url(parameters.text("url").unwrap_or_default()).map_err(failed)?;
        let pairs = policy::query(parameters.text("query").unwrap_or_default()).map_err(failed)?;
        if !pairs.is_empty() {
            let mut query = url.query_pairs_mut();
            for (name, value) in &pairs {
                query.append_pair(name, value);
            }
        }
        let headers =
            policy::headers(parameters.text("headers").unwrap_or_default()).map_err(failed)?;
        let method: reqwest::Method = parameters
            .text("method")
            .unwrap_or("GET")
            .parse()
            .map_err(|_| fail("invalid-parameters", "unknown method"))?;
        let body = match parameters.text("body").filter(|b| !b.trim().is_empty()) {
            Some(body) if parameters.text("bodyType") == Some("text") => {
                Some((body.to_owned(), "text/plain; charset=utf-8"))
            }
            Some(body) => {
                serde_json::from_str::<serde_json::Value>(body)
                    .map_err(|e| fail(codes::INVALID_BODY, format!("the body is not JSON: {e}")))?;
                Some((body.to_owned(), "application/json"))
            }
            None => None,
        };
        let request = client::Request {
            method: method.clone(),
            url,
            headers,
            body,
            auth: Self::auth(parameters, context)?,
            timeout: Self::timeout(parameters, 300),
            follow_redirects: parameters.boolean("followRedirects").unwrap_or(true),
            max_bytes: self.settings.max_bytes,
        };
        let response = self.send(request, context)?;
        let text = String::from_utf8_lossy(&response.body).into_owned();
        let shown = policy::shown_url(&response.final_url);
        let message = format!(
            "{method} {shown}: {} in {} ms",
            response.status,
            response.elapsed.as_millis()
        );
        let ok = (200..300).contains(&response.status);
        let mut result = if !ok
            && response.status >= 400
            && parameters.boolean("failOnHttpError").unwrap_or(true)
        {
            ActionResult::error(codes::HTTP_STATUS, message)
        } else {
            ActionResult::success(message)
        };
        if let Some(pointer) = optional(parameters, "extract") {
            let json: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
                fail(
                    codes::INVALID_RESPONSE,
                    "the response is not JSON, so nothing can be read from it",
                )
            })?;
            if let Some(value) = json.pointer(pointer) {
                let value = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                result = result.with_data("value", value);
            }
        }
        let headers: Vec<String> = response
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect();
        Ok(result
            .with_data("status", response.status.to_string())
            .with_data("ok", ok.to_string())
            .with_data("contentType", response.content_type.clone())
            .with_data("headers", headers.join("\n"))
            .with_data("size", response.body.len().to_string())
            .with_data("truncated", response.truncated.to_string())
            .with_data("body", text)
            .with_data("url", shown)
            .with_data("redirects", response.redirects.to_string())
            .with_data("durationMs", response.elapsed.as_millis().to_string()))
    }

    fn health_check(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ActionResult, Failure> {
        let url =
            policy::request_url(parameters.text("url").unwrap_or_default()).map_err(failed)?;
        let expected = parameters.text("expectedStatus").unwrap_or("2xx");
        status_matches(expected, 200).ok_or_else(|| {
            fail(
                "invalid-parameters",
                format!("“{expected}” is not a status such as 200, 2xx or 200-299"),
            )
        })?;
        let shown = policy::shown_url(&url);
        let begun = std::time::Instant::now();
        let request = client::Request {
            method: reqwest::Method::GET,
            url,
            headers: Vec::new(),
            body: None,
            auth: Self::auth(parameters, context)?,
            timeout: Self::timeout(parameters, 120),
            follow_redirects: true,
            max_bytes: self.settings.max_bytes.min(256 * 1024),
        };
        let outcome = match self.send(request, context) {
            Ok(response) => {
                let body = String::from_utf8_lossy(&response.body);
                let status_ok = status_matches(expected, response.status).unwrap_or(false);
                let contains = optional(parameters, "contains");
                let reason = if status_ok {
                    contains
                        .filter(|n| !body.contains(*n))
                        .map(|needle| format!("the response does not contain “{needle}”"))
                } else {
                    Some(format!("status {} is not {expected}", response.status))
                };
                (Some(response.status), reason)
            }
            // Cancellation and refused addresses are not a health result.
            Err(failure)
                if matches!(
                    failure.code.as_deref(),
                    Some(
                        "cancelled"
                            | "not-permitted"
                            | codes::BLOCKED_ADDRESS
                            | "credential-missing"
                            | codes::INSECURE_CREDENTIALS
                    )
                ) =>
            {
                return Err(failure);
            }
            Err(failure) => (None, Some(failure.message)),
        };
        let elapsed = begun.elapsed().as_millis().to_string();
        let (status, reason) = outcome;
        let mut result = match &reason {
            None => ActionResult::success(format!("{shown} is healthy")),
            Some(reason) if parameters.boolean("failWhenUnhealthy").unwrap_or(true) => {
                ActionResult::error(
                    codes::UNHEALTHY,
                    format!("{shown} is not healthy: {reason}"),
                )
            }
            Some(reason) => ActionResult::success(format!("{shown} is not healthy: {reason}")),
        };
        result = result
            .with_data("healthy", reason.is_none().to_string())
            .with_data("durationMs", elapsed);
        if let Some(status) = status {
            result = result.with_data("status", status.to_string());
        }
        if let Some(reason) = reason {
            result = result.with_data("reason", reason);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
