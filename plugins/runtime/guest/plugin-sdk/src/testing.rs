//! Native unit tests for plugins: run actions without wasm, with a
//! scripted stand-in for the agent's broker and executors.
//!
//! ```
//! use deckpilot_plugin_sdk::prelude::*;
//! use deckpilot_plugin_sdk::testing::MockHost;
//! use deckpilot_plugin_sdk::{CallResult, CapabilityRequest};
//!
//! fn ping(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
//!     let r = ctx.http_get("https://api.example.com/ping")?;
//!     Ok(Output::ok().with("status", r.status))
//! }
//! let plugin = Plugin::new("acme.ping", "1.0.0").action(Action::new("ping", ping));
//!
//! let mut denied = MockHost::deny_all();
//! let out = denied.invoke(&plugin, "ping", serde_json::json!({}));
//! assert!(matches!(out, ActionOutcome::Failed { ref code, .. } if code == "not-permitted"));
//!
//! let mut host = MockHost::new(|_req: &CapabilityRequest| {
//!     CallResult::Ok(serde_json::json!({"status": 204}))
//! });
//! let out = host.invoke(&plugin, "ping", serde_json::json!({}));
//! assert!(matches!(out, ActionOutcome::Ok { .. }));
//! assert_eq!(host.requests().len(), 1);
//! ```

use std::fmt;

use deckpilot_plugin_abi::{ActionOutcome, CallErrorCode, CallResult, CapabilityRequest, Params};

use crate::Plugin;
use crate::context::Transport;

type Responder = Box<dyn FnMut(&CapabilityRequest) -> CallResult>;

/// Answers capability requests like the agent would, and records them.
pub struct MockHost {
    respond: Responder,
    requests: Vec<CapabilityRequest>,
}

impl fmt::Debug for MockHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockHost")
            .field("requests", &self.requests)
            .finish_non_exhaustive()
    }
}

impl MockHost {
    pub fn new(respond: impl FnMut(&CapabilityRequest) -> CallResult + 'static) -> Self {
        Self {
            respond: Box::new(respond),
            requests: Vec::new(),
        }
    }

    /// Every request succeeds with `null` (an empty HTTP 200 for fetches).
    pub fn allow_all() -> Self {
        Self::new(|req| match req {
            CapabilityRequest::HttpFetch(_) => CallResult::Ok(serde_json::json!({"status": 200})),
            _ => CallResult::Ok(serde_json::Value::Null),
        })
    }

    /// Every request is denied like an ungranted permission (logs too).
    pub fn deny_all() -> Self {
        Self::new(|_| CallResult::error(CallErrorCode::NotPermitted, "not granted"))
    }

    /// Requests made so far, in order.
    pub fn requests(&self) -> &[CapabilityRequest] {
        &self.requests
    }

    /// Runs one action as the host would. `params` is a JSON object.
    ///
    /// # Panics
    /// If `params` is not a valid parameter object.
    pub fn invoke(
        &mut self,
        plugin: &Plugin,
        action: &str,
        params: serde_json::Value,
    ) -> ActionOutcome {
        let params: Params =
            serde_json::from_value(params).expect("params must be a JSON object of values");
        self.invoke_params(plugin, action, &params)
    }

    pub fn invoke_params(
        &mut self,
        plugin: &Plugin,
        action: &str,
        params: &Params,
    ) -> ActionOutcome {
        plugin.run(self, action, params)
    }
}

impl Transport for MockHost {
    fn call(&mut self, request: &CapabilityRequest) -> CallResult {
        self.requests.push(request.clone());
        (self.respond)(request)
    }
}
