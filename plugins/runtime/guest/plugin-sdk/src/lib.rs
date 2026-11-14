//! # DeckPilot plugin SDK for Rust
//!
//! Write a DeckPilot plugin as a `cdylib` for `wasm32-unknown-unknown`:
//! declare the plugin and its actions with typed parameters, return
//! structured results or typed errors, and reach the outside world only
//! through capabilities that the permission broker checks on every call.
//!
//! ```
//! use deckpilot_plugin_sdk::prelude::*;
//!
//! fn plugin() -> Plugin {
//!     Plugin::new("acme.hello", "0.1.0").action(
//!         Action::new("greet", greet)
//!             .param(Param::text("name").required().max_length(64))
//!             .param(Param::choice("style", &["plain", "excited"]).default("plain")),
//!     )
//! }
//!
//! fn greet(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
//!     let name: String = args.get("name")?;
//!     let style: String = args.get("style")?;
//!     ctx.info("greeting someone");
//!     let mark = if style == "excited" { "!" } else { "." };
//!     Ok(Output::message(format!("Hello, {name}{mark}")).with("name", name))
//! }
//!
//! // In the plugin crate (a cdylib): generates the guest ABI exports.
//! // deckpilot_plugin_sdk::plugin!(plugin);
//! # let mut host = deckpilot_plugin_sdk::testing::MockHost::allow_all();
//! # let out = host.invoke(&plugin(), "greet", serde_json::json!({"name": "Ada"}));
//! # assert!(matches!(out, ActionOutcome::Ok { .. }));
//! ```
//!
//! ## What the SDK does for you
//!
//! * **Exports** ([`plugin!`]): `dp_abi_version`, `dp_alloc`, `dp_invoke`
//!   and the optional `dp_describe`, `dp_init`, `dp_shutdown` of guest
//!   ABI 1. The plugin crate contains no `unsafe` of its own; the macro's
//!   export attributes need `unsafe_code` to be `deny` rather than `forbid`.
//! * **Parameters** ([`Param`], [`Args`]): the declared kinds (`text`,
//!   `integer`, `number`, `boolean`, `choice`, `secret`) are checked, ranges
//!   and options enforced, defaults filled, unknown parameters refused, all
//!   before your handler runs. A bad invocation becomes the typed error
//!   [`codes::INVALID_PARAMETERS`].
//! * **Results** ([`Output`], [`PluginError`]): `ok` with message and data,
//!   `skipped`, or `failed` with a code. Codes are checked; a code outside
//!   `[a-z][a-z0-9-]*` becomes [`codes::INVALID_RESULT`].
//! * **Capabilities** ([`Context`]): HTTP to an exact origin, notifications,
//!   per-plugin storage and log, all brokered. A denial is a
//!   [`CapabilityError`] that says what was denied and how to declare it;
//!   `?` turns it into a [`PluginError`] with a shared code.
//! * **Secrets** ([`SecretHandle`]): credential slots are handles. The
//!   secret is injected by the agent into requests to the slot's origins
//!   and never enters plugin memory.
//! * **Lifecycle** ([`Plugin::on_init`], [`Plugin::on_shutdown`]): init runs
//!   once when the host loads the module (or before the first invocation on
//!   a host without `dp_init`); a failing init refuses the load.
//!
//! ## Runtime facts
//!
//! * `wasm32-unknown-unknown` std has no file system, network, clock,
//!   environment or process; such calls fail inside the guest.
//! * A panic aborts (the `guest` profile has `panic = "abort"`): the host
//!   process exits and the supervisor restarts it with backoff. Return
//!   errors instead.
//! * Every invocation has a wall-clock timeout, a CPU deadline between host
//!   calls and a fuel budget; exceeding them ends the host process.
//! * Off wasm (unit tests, `cargo check`) the SDK compiles; capability calls
//!   then fail with [`CapabilityErrorKind::Unavailable`] unless you run the
//!   plugin through [`testing::MockHost`].
//!
//! ## Versioning
//!
//! | Number | Value | Meaning |
//! | --- | --- | --- |
//! | [`SDK_VERSION`] | this crate's semver | breaking Rust API changes bump the major (the minor while 0.x) |
//! | [`API_LEVEL`] | 1.0 | plugin API level (design §6.5) this SDK implements |
//! | [`ABI_VERSION`] | 1 | guest ABI; checked at compile time |
#![deny(unsafe_code)]

mod args;
mod context;
mod definition;
mod output;
pub mod testing;

pub use args::{Args, FromArg};
pub use context::{
    CapabilityError, CapabilityErrorKind, Context, MAX_LOG_LEN, Request, Response, SecretHandle,
};
pub use definition::{Action, DefinitionError, Handler, Param, Plugin};
pub use output::{ActionResult, IntoValue, Output, PluginError, codes};

pub use deckpilot_plugin_abi::capability::{AuthScheme, Header, LogLevel};
pub use deckpilot_plugin_abi::describe::{ParamKind, PluginDescriptor};
pub use deckpilot_plugin_abi::{
    ActionOutcome, CallErrorCode, CallResult, CapabilityRequest, HttpMethod, ParamValue, Params,
};

/// The guest ABI this SDK implements.
pub const ABI_VERSION: u32 = deckpilot_plugin_abi::ABI_VERSION;
const _: () = assert!(
    ABI_VERSION == 1,
    "deckpilot-plugin-sdk implements guest ABI 1"
);

/// This crate's version.
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// This crate's name, as reported in the descriptor (`<name>/<version>`).
pub const SDK_NAME: &str = env!("CARGO_PKG_NAME");

/// Plugin API level (design §6.5): `major.minor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ApiLevel {
    pub major: u16,
    pub minor: u16,
}

impl std::fmt::Display for ApiLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// The plugin API level this SDK implements. A manifest built with this
/// SDK declares `"engines": { "pluginApi": "^1.0" }`.
pub const API_LEVEL: ApiLevel = ApiLevel { major: 1, minor: 0 };

/// Everything a plugin usually needs.
pub mod prelude {
    pub use crate::{
        Action, ActionOutcome, ActionResult, Args, AuthScheme, CapabilityError,
        CapabilityErrorKind, Context, HttpMethod, LogLevel, Output, Param, Plugin, PluginError,
        Request, Response, SecretHandle, codes,
    };
}

/// Generates the guest ABI 1 exports for a plugin built by `$build`
/// (a `fn() -> Plugin`). Use it once, at the root of a `cdylib` crate.
///
/// The plugin is built on first use and kept for the life of the module.
#[macro_export]
macro_rules! plugin {
    ($build:path) => {
        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn dp_abi_version() -> i32 {
            $crate::ABI_VERSION as i32
        }

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn dp_alloc(len: i32) -> i32 {
            $crate::__rt::alloc(len)
        }

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn dp_invoke(ptr: i32, len: i32) -> i64 {
            // SAFETY: the host passes a buffer it obtained from dp_alloc.
            unsafe { $crate::__rt::invoke($crate::__rt::registered($build), ptr, len) }
        }

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn dp_describe() -> i64 {
            $crate::__rt::describe($crate::__rt::registered($build))
        }

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn dp_init() -> i32 {
            $crate::__rt::init($crate::__rt::registered($build))
        }

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn dp_shutdown() {
            $crate::__rt::shutdown($crate::__rt::registered($build))
        }
    };
}

/// Runtime support for [`plugin!`]. Not a stable API.
#[doc(hidden)]
pub mod __rt {
    use std::sync::OnceLock;

    use deckpilot_plugin_abi::guest::InvokeInput;
    use deckpilot_plugin_guest::__rt as raw;

    use crate::context::WasmTransport;
    use crate::{ActionOutcome, Plugin, codes};

    static PLUGIN: OnceLock<Plugin> = OnceLock::new();

    pub fn registered(build: fn() -> Plugin) -> &'static Plugin {
        PLUGIN.get_or_init(build)
    }

    pub fn alloc(len: i32) -> i32 {
        raw::alloc(len)
    }

    /// Body of `dp_invoke`.
    ///
    /// # Safety
    /// `ptr`/`len` must describe a buffer obtained from [`alloc`].
    #[allow(unsafe_code)]
    pub unsafe fn invoke(plugin: &Plugin, ptr: i32, len: i32) -> i64 {
        // SAFETY: forwarded contract.
        let input = unsafe { raw::take_input(ptr, len) };
        let outcome = match serde_json::from_slice::<InvokeInput>(&input) {
            Ok(i) => plugin.run(&mut WasmTransport, &i.action, &i.params),
            Err(_) => ActionOutcome::failed(
                codes::INVALID_PARAMETERS,
                "the invocation input did not parse",
            ),
        };
        raw::publish_output(serde_json::to_vec(&outcome).unwrap_or_default())
    }

    /// Body of `dp_describe`.
    pub fn describe(plugin: &Plugin) -> i64 {
        raw::publish_output(serde_json::to_vec(&plugin.descriptor()).unwrap_or_default())
    }

    /// Body of `dp_init`: 0 ready, 1 init hook failed, 2 invalid definition.
    pub fn init(plugin: &Plugin) -> i32 {
        if plugin.validate().is_err() {
            return 2;
        }
        match plugin.ensure_init() {
            Ok(()) => 0,
            Err(_) => 1,
        }
    }

    /// Body of `dp_shutdown`.
    pub fn shutdown(plugin: &Plugin) {
        plugin.run_shutdown();
    }
}
