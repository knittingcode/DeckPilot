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
//!   and the optional `dp_describe`, `dp_init`, `dp_shutdown` and
//!   `dp_features` of guest ABI 1 (the module reads the network kind of a
//!   failed fetch: [`CapabilityError::network_kind`]). The plugin crate contains no `unsafe` of its own; the macro's
//!   export attributes need `unsafe_code` to be `deny` rather than `forbid`.
//! * **Parameters** ([`Param`], [`Args`]): the declared kinds (`text`,
//!   `integer`, `number`, `boolean`, `choice`, `secret`) are checked, ranges
//!   and options enforced, defaults filled, unknown parameters refused, all
//!   before your handler runs. A bad invocation becomes the typed error
//!   [`codes::INVALID_PARAMETERS`].
//! * **Results** ([`Output`], [`PluginError`]): `ok` with message and data,
//!   `skipped`, or `failed` with a code. Codes are checked; a code outside
//!   `[a-z][a-z0-9-]*` becomes [`codes::INVALID_RESULT`].
//! * **Capabilities** ([`Context`]): HTTP to an exact origin
//!   ([`Context::fetch`]), notifications ([`Context::notify`]), per-plugin
//!   storage, settings, own variables ([`Context::publish_variable`]) and
//!   log ([`Context::log`]), all brokered. A denial is a
//!   [`CapabilityError`] that says what was denied and how to declare it;
//!   `?` turns it into a [`PluginError`] with a shared code.
//! * **Secrets** ([`SecretHandle`]): credential slots are handles. The
//!   secret is injected by the agent into requests to the slot's origins
//!   and never enters plugin memory; [`Context::has_credential`] only says
//!   whether the user stored one.
//! * **Storage** ([`Context::storage_get`] and friends): the plugin's own
//!   key-value store in the agent's encrypted plugin database, 256 KiB and
//!   1,000 keys by default (a `storage` permission raises the bytes to its
//!   `quotaKb`), keys at most 128 characters, values at most 64 KiB of
//!   JSON ([`Context::storage_usage`]).
//! * **Settings** ([`Context::settings`]): the values of the manifest's
//!   `contributes.settings`, validated by the agent, defaults filled in.
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
//! | [`API_LEVEL`] | 1.0 | plugin API level (design §6.5) a plugin needs by default |
//! | [`LATEST_API_LEVEL`] | 1.6 | highest plugin API level this SDK implements |
//! | [`ABI_VERSION`] | 1 | guest ABI; checked at compile time |
#![deny(unsafe_code)]

mod args;
mod context;
mod controls;
mod definition;
pub mod dialog;
mod output;
pub mod testing;

pub use args::{Args, FromArg};
pub use context::{
    CapabilityError, CapabilityErrorKind, Context, MAX_LOG_LEN, MAX_RANDOM_BYTES, NotifyDelivery,
    Now, Request, Response, RunCause, RunContext, RunSummary, RunTrigger, SecretHandle, Settings,
    StorageUsage, TranslateArg,
};
pub use controls::{Control, Controls, Input, MAX_INPUTS_PER_CONTROL};
pub use definition::{Action, DefinitionError, Handler, Param, Plugin, StateHandler};
pub use output::{ActionResult, IntoValue, Output, PluginError, codes};

pub use deckpilot_plugin_abi::capability::{AuthScheme, Header, LogLevel, NotifySeverity};
pub use deckpilot_plugin_abi::describe::{ParamKind, PluginDescriptor};
pub use deckpilot_plugin_abi::state::{
    KeyState, StateAccent as Accent, StateFailure, StateGlyph as Glyph,
    StateIndicator as Indicator, StateOutcome,
};
pub use deckpilot_plugin_abi::{
    ActionOutcome, CallErrorCode, CallResult, CapabilityRequest, HttpMethod, NetworkErrorKind,
    ParamValue, Params,
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

/// The plugin API level a plugin built with this SDK needs unless it
/// says otherwise: every agent with plugins provides it, and its manifest
/// declares `"engines": { "pluginApi": "^1.0" }`. A plugin that cannot
/// work without a later addition raises its own level with
/// [`Plugin::requires_api`] (see [`Plugin::api_level`]).
pub const API_LEVEL: ApiLevel = ApiLevel { major: 1, minor: 0 };

/// The highest plugin API level this SDK implements: 1.1 adds
/// [`Context::now`] and [`Context::random_bytes`], 1.2 key state
/// ([`Action::state`]), 1.3 the controls an action declares
/// ([`Action::controls`]), 1.4 nothing for the guest (manifest
/// dependencies), 1.5 standard dialogs ([`Context::dialog`]), 1.6 the
/// app's language and the plugin's own translations ([`Context::locale`],
/// [`Context::t`]; `locales` in the manifest).
pub const LATEST_API_LEVEL: ApiLevel = ApiLevel { major: 1, minor: 6 };

/// The plugin API level that introduced [`Context::locale`],
/// [`Context::t`] and the manifest's `locales`: a plugin that uses them
/// needs it.
pub const LOCALIZATION_API_LEVEL: ApiLevel = ApiLevel { major: 1, minor: 6 };

/// The plugin API level that introduced [`Context::dialog`]: a plugin that
/// shows dialogs needs it (and the `dialogs` permission).
pub const DIALOGS_API_LEVEL: ApiLevel = ApiLevel { major: 1, minor: 5 };

/// The plugin API level that introduced [`Action::controls`]: a plugin
/// declaring controls needs it.
pub const CONTROLS_API_LEVEL: ApiLevel = ApiLevel { major: 1, minor: 3 };

/// Everything a plugin usually needs.
pub mod prelude {
    pub use crate::{
        Accent, Action, ActionOutcome, ActionResult, Args, AuthScheme, CapabilityError,
        CapabilityErrorKind, Context, Control, Controls, Glyph, HttpMethod, Indicator, Input,
        KeyState, LogLevel, NetworkErrorKind, NotifyDelivery, NotifySeverity, Now, Output, Param,
        Plugin, PluginError, Request, Response, RunCause, RunContext, RunTrigger, SecretHandle,
        Settings, StateOutcome, StorageUsage, codes,
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

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn dp_features() -> i64 {
            $crate::__rt::FEATURES as i64
        }

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn dp_state(ptr: i32, len: i32) -> i64 {
            // SAFETY: the host passes a buffer it obtained from dp_alloc.
            unsafe { $crate::__rt::key_state($crate::__rt::registered($build), ptr, len) }
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

    /// What `dp_features` reports: this SDK reads every addition of its
    /// ABI crate (the network kind of a failed fetch, the delivery of a
    /// notification).
    pub const FEATURES: u64 = deckpilot_plugin_abi::guest::features::ALL;

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

    /// Body of `dp_state`.
    ///
    /// # Safety
    /// `ptr`/`len` must describe a buffer obtained from [`alloc`].
    #[allow(unsafe_code)]
    pub unsafe fn key_state(plugin: &Plugin, ptr: i32, len: i32) -> i64 {
        // SAFETY: forwarded contract.
        let input = unsafe { raw::take_input(ptr, len) };
        let outcome = match serde_json::from_slice::<InvokeInput>(&input) {
            Ok(i) => plugin.key_state(&mut WasmTransport, &i.action, &i.params),
            Err(_) => crate::StateOutcome::failed(
                codes::INVALID_PARAMETERS,
                "the state input did not parse",
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
