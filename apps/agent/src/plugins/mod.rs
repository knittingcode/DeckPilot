//! Plugins (experimental; `docs/marketplace/agent-integration.md`).
//!
//! The plugin requests of IPC protocol 10 arrive here. Without the
//! `plugins` feature (every release build) they are answered with
//! "plugins not available", and `getPluginRuntime` says why. With it, and
//! with plugins enabled (`--enable-plugins`), they go to a worker thread
//! that owns the plugin store, the registry client and the permission
//! broker, and one lane thread per running plugin with its supervisor
//! ([`PluginRuntime`]). Installed plugins are also feature modules of the
//! agent's module registry ([`PluginRuntime::modules`], `engine.rs`), so
//! keys and workflows run their actions. Plugin code never runs in the
//! agent: every invocation goes through the supervisor to a sandboxed
//! `deckpilot-plugin-host` process.

use std::path::PathBuf;

use deckpilot_ipc::plugins::{PluginRuntimeStatus, reason};
use deckpilot_ipc::{ErrorCode, IpcError, Request, Response};
use tokio::sync::oneshot;

#[cfg(feature = "plugins")]
mod bundled;
#[cfg(feature = "plugins")]
mod convert;
#[cfg(feature = "plugins")]
mod data;
#[cfg(feature = "plugins")]
mod engine;
#[cfg(feature = "plugins")]
mod executor;
#[cfg(feature = "plugins")]
mod fetch;
#[cfg(feature = "plugins")]
pub mod key;
#[cfg(feature = "plugins")]
mod lane;
#[cfg(feature = "plugins")]
mod migration;
#[cfg(feature = "plugins")]
mod registry;
#[cfg(feature = "plugins")]
mod service;
#[cfg(feature = "plugins")]
mod variables;
#[cfg(feature = "plugins")]
mod worker;

/// Whether this agent was built with the `plugins` feature.
pub const BUILT: bool = cfg!(feature = "plugins");

/// Environment variable that enables plugins like `--enable-plugins` (`1`).
pub const ENABLE_ENV: &str = "DECKPILOT_ENABLE_PLUGINS";
/// Environment variable naming the plugin host executable (development;
/// an installed application finds the host next to the agent).
pub const HOST_ENV: &str = "DECKPILOT_PLUGIN_HOST";
/// Environment variable naming a `bundled` directory of first-party
/// plugins (development and test builds only; an installed application
/// uses the one in its resources).
pub const BUNDLED_ENV: &str = "DECKPILOT_BUNDLED_PLUGINS";
/// Environment variable naming the local registry directory.
pub const REGISTRY_ENV: &str = "DECKPILOT_PLUGIN_REGISTRY";
/// Environment variable naming the registry's root keys (`roots.json`).
pub const REGISTRY_ROOTS_ENV: &str = "DECKPILOT_PLUGIN_REGISTRY_ROOTS";
/// Environment variable with the https URL of a registry site.
pub const REGISTRY_URL_ENV: &str = "DECKPILOT_PLUGIN_REGISTRY_URL";
/// Environment variable with the https URL packages are downloaded from.
pub const REGISTRY_RELEASES_URL_ENV: &str = "DECKPILOT_PLUGIN_REGISTRY_RELEASES_URL";

/// How plugins are set up for this agent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginSettings {
    /// Plugin requests are answered (`--enable-plugins`). Off by default.
    pub enabled: bool,
    /// The `deckpilot-plugin-host` executable (development). Default: next
    /// to the agent's own executable, where the installers put it.
    pub host: Option<PathBuf>,
    /// The `bundled` directory of first-party plugins (`--bundled-plugins`,
    /// honoured by debug and test builds only). Default: the application's
    /// resources (`deckpilot_plugin_bundled::locate`).
    pub bundled: Option<PathBuf>,
    /// A local registry: a directory with the static site in `site/v1/`
    /// and the release assets in `releases/` (the layout the publishing
    /// tool writes).
    pub registry: Option<PathBuf>,
    /// A registry served over https: the URL of its site (documents at
    /// `<url>v1/…`). Exclusive with [`PluginSettings::registry`]. With
    /// root keys and neither set, the official site
    /// (`registry::DEFAULT_REGISTRY_URL`); without root keys, none.
    pub registry_url: Option<String>,
    /// Where packages are downloaded from (https; `<url><id>--v<version>/
    /// <file>`), instead of the release URL the registry pins. The bytes
    /// must still be the pinned ones.
    pub registry_releases_url: Option<String>,
    /// The registry's root role (`roots.json`), for builds without embedded
    /// roots (`DECKPILOT_REGISTRY_ROOT_PUBKEYS` at build time) and for
    /// development: it overrides embedded roots in debug and `test-hooks`
    /// builds only (`deckpilot_plugin_registry::roots::choose`). It must
    /// not come from the registry itself.
    pub registry_roots: Option<PathBuf>,
}

impl PluginSettings {
    /// Settings from the environment ([`ENABLE_ENV`], [`HOST_ENV`],
    /// [`REGISTRY_ENV`], [`REGISTRY_URL_ENV`], [`REGISTRY_RELEASES_URL_ENV`],
    /// [`REGISTRY_ROOTS_ENV`]); command-line options override them.
    pub fn from_env() -> Self {
        let path = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let text = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        Self {
            enabled: std::env::var(ENABLE_ENV).is_ok_and(|v| v == "1"),
            host: path(HOST_ENV),
            bundled: path(BUNDLED_ENV),
            registry: path(REGISTRY_ENV),
            registry_url: text(REGISTRY_URL_ENV),
            registry_releases_url: text(REGISTRY_RELEASES_URL_ENV),
            registry_roots: path(REGISTRY_ROOTS_ENV),
        }
    }
}

/// The pins of the bundled first-party plugins compiled into this agent
/// (`deckpilot-agent --print-bundled-pins`, for the release check
/// `tools/release-pipeline/check-bundled-plugins.sh`). A build without the
/// `plugins` feature bundles nothing.
pub fn embedded_pins() -> &'static [u8] {
    #[cfg(feature = "plugins")]
    {
        deckpilot_plugin_bundled::EMBEDDED_PINS
    }
    #[cfg(not(feature = "plugins"))]
    {
        b"{\"pinsVersion\":1,\"plugins\":[]}\n"
    }
}

/// Whether plugins may run here: only where the host has an OS sandbox
/// (macOS: Seatbelt; Linux: seccomp and Landlock; Windows: an `AppContainer`
/// with a Job Object, launched by the supervisor). On every platform each
/// started host must also report a **full** sandbox before it receives a
/// plugin (`service::host_config`); a host reporting `partial` or `none`
/// is stopped and the invocation fails. Elsewhere installing and
/// inspecting work, enabling and running are refused.
pub fn execution_supported() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))
}

/// `macos`, `linux` or `windows` (or the OS name elsewhere).
pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else {
        std::env::consts::OS
    }
}

/// Why plugins cannot run on this platform, if they cannot.
pub fn platform_problem() -> Option<String> {
    (!execution_supported()).then(|| {
        format!(
            "plugins cannot run on {}: the plugin host has no OS sandbox on this platform \
             (plugins run only on macOS, Linux and Windows, in a full sandbox); plugins can be \
             installed and inspected, but not enabled or run",
            platform()
        )
    })
}

/// The built-in modules with their stored states, for the agent's module
/// registry. With the `plugins` feature and `test-hooks`, an agent test
/// can add the migration test fixture (`migration/fixture.rs`); every
/// other build gets exactly `deckpilot_modules::configured`.
pub(crate) fn configured_modules(
    states: &deckpilot_core::ModuleStates,
) -> (deckpilot_core::ModuleRegistry, Vec<String>) {
    #[cfg(feature = "plugins")]
    {
        migration::configured(states)
    }
    #[cfg(not(feature = "plugins"))]
    {
        deckpilot_modules::configured(states)
    }
}

/// Requests of protocol 10's plugin API.
pub(crate) fn is_plugin_request(request: &Request) -> bool {
    matches!(
        request,
        Request::GetPluginRuntime
            | Request::ListPlugins
            | Request::GetPlugin { .. }
            | Request::ListPluginsPaged
            | Request::GetPluginPaged { .. }
            | Request::GetPluginActions { .. }
            | Request::GetPluginIcon { .. }
            | Request::ListAvailablePlugins { .. }
            | Request::InstallPlugin { .. }
            | Request::EnablePlugin { .. }
            | Request::DisablePlugin { .. }
            | Request::GrantPluginPermissions { .. }
            | Request::RevokePluginPermission { .. }
            | Request::UpdatePlugin { .. }
            | Request::ApprovePluginUpdate { .. }
            | Request::DeclinePluginUpdate { .. }
            | Request::RollbackPlugin { .. }
            | Request::UninstallPlugin { .. }
            | Request::InvokePluginAction { .. }
            | Request::ListKnownPlugins
            | Request::GetPluginSettings { .. }
            | Request::SetPluginSettings { .. }
            | Request::ListPluginCredentials { .. }
            | Request::SetPluginCredential { .. }
            | Request::DeletePluginCredential { .. }
            | Request::ListModuleMigrations
            | Request::DecideModuleMigration { .. }
            | Request::RetryModuleMigration { .. }
    )
}

fn unavailable_reason(settings: &PluginSettings) -> String {
    if !BUILT {
        "plugins are not available: this agent was built without plugin support".into()
    } else if !settings.enabled {
        "plugins are not enabled in this agent (start it with --enable-plugins)".into()
    } else {
        "plugins are not available right now".into()
    }
}

/// The error of every plugin request but `getPluginRuntime` while plugins
/// cannot be used.
pub(crate) fn unavailable(settings: &PluginSettings) -> IpcError {
    IpcError::new(ErrorCode::Unsupported, unavailable_reason(settings))
        .with_reason(reason::PLUGINS_UNAVAILABLE)
}

#[cfg(feature = "plugins")]
pub(crate) fn locked() -> IpcError {
    IpcError::new(
        ErrorCode::Locked,
        "the configuration is locked; plugins are available once it is unlocked",
    )
    .with_reason(reason::LOCKED)
}

/// The status when no plugin worker answers (not built, not enabled).
fn idle_status(settings: &PluginSettings) -> PluginRuntimeStatus {
    PluginRuntimeStatus {
        available: false,
        built: BUILT,
        enabled: settings.enabled && BUILT,
        execution_supported: execution_supported(),
        platform: platform().to_owned(),
        reason: Some(unavailable_reason(settings)),
        registry: None,
        // Nothing beyond the refusal: no plugin request is answered.
        features: Vec::new(),
    }
}

/// What the plugin worker can do beyond the original plugin requests
/// (`PluginRuntimeStatus::features`).
#[cfg(feature = "plugins")]
pub(crate) fn features() -> Vec<String> {
    vec![deckpilot_ipc::plugins::feature::PAGED_ANSWERS.to_owned()]
}

type Reply = oneshot::Sender<Result<Response, IpcError>>;

/// The agent's plugin runtime: settings, and with the `plugins` feature
/// the worker that serves plugin requests.
#[derive(Debug, Default)]
pub(crate) struct PluginRuntime {
    settings: PluginSettings,
    #[cfg(feature = "plugins")]
    worker: Option<worker::PluginWorker>,
}

// Without the feature these only answer "not available".
#[cfg_attr(
    not(feature = "plugins"),
    allow(clippy::unused_self, clippy::needless_pass_by_value)
)]
impl PluginRuntime {
    /// Starts the worker when built with plugins and enabled. `changed` is
    /// called (on the worker's thread) whenever the installed plugins or
    /// their availability changed, so the agent registers
    /// [`PluginRuntime::modules`] again.
    pub(crate) fn new(
        settings: PluginSettings,
        data_dir: &std::path::Path,
        events: &tokio::sync::broadcast::Sender<deckpilot_ipc::AgentEvent>,
        plugin_events: &tokio::sync::broadcast::Sender<deckpilot_ipc::plugins::PluginEvent>,
        notifier: std::sync::Arc<dyn deckpilot_core::notify::Notifier>,
        changed: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        #[cfg(feature = "plugins")]
        {
            let worker = settings.enabled.then(|| {
                worker::PluginWorker::start(
                    settings.clone(),
                    data_dir,
                    worker::Events {
                        agent: events.clone(),
                        plugins: plugin_events.clone(),
                        notifier,
                    },
                    changed,
                )
            });
            Self { settings, worker }
        }
        #[cfg(not(feature = "plugins"))]
        {
            let _ = (data_dir, events, plugin_events, notifier, changed);
            Self { settings }
        }
    }

    /// One feature module per installed plugin, for the agent's module
    /// registry: enabled when its actions can run, otherwise disabled with
    /// the reason. Empty without the `plugins` feature, with plugins off,
    /// and while locked.
    pub(crate) fn modules(&self) -> Vec<Box<dyn deckpilot_core::FeatureModule>> {
        #[cfg(feature = "plugins")]
        if let Some(worker) = &self.worker {
            let invoker = worker.invoker();
            return worker
                .catalog()
                .into_iter()
                .filter_map(|plugin| {
                    engine::PluginModule::new(plugin, invoker.clone(), worker.values())
                })
                .map(|module| Box::new(module) as Box<dyn deckpilot_core::FeatureModule>)
                .collect();
        }
        Vec::new()
    }

    /// Hands the module migration engine what `deckpilot.db` says about
    /// the modules with an alias block (`modules`: the built-in modules
    /// just loaded, before plugins are registered). Reads `deckpilot.db`,
    /// never writes it; nothing is read while no alias exists. The first
    /// snapshot after the plugin store opened runs the migration.
    pub(crate) fn migrate(
        &self,
        store: &deckpilot_storage::SqliteStore,
        modules: &deckpilot_core::ModuleRegistry,
    ) {
        #[cfg(feature = "plugins")]
        if let Some(worker) = &self.worker {
            match migration::LegacySnapshot::read(store, modules) {
                Ok(snapshot) if snapshot.modules.is_empty() => {}
                Ok(snapshot) => worker.migrate(snapshot),
                Err(error) => {
                    tracing::warn!(%error, "the modules that can migrate cannot be read");
                }
            }
        }
        #[cfg(not(feature = "plugins"))]
        let _ = (store, modules);
    }

    /// Where the legacy action types of migrated modules go
    /// (`docs/marketplace/module-migration.md` §5.2): from the migration
    /// journal while the plugin store is open; every module to its
    /// built-in otherwise, and without the `plugins` feature.
    pub(crate) fn legacy_routing(&self) -> deckpilot_core::alias::LegacyRouting {
        #[cfg(feature = "plugins")]
        if let Some(worker) = &self.worker {
            return worker.legacy_routing();
        }
        deckpilot_core::alias::LegacyRouting::default()
    }

    /// Answers a plugin request (here or on the worker).
    pub(crate) fn handle(&self, request: Request, reply: Reply, unlocked: bool) {
        #[cfg(feature = "plugins")]
        if let Some(worker) = &self.worker {
            if !unlocked && !matches!(request, Request::GetPluginRuntime) {
                let _ = reply.send(Err(locked()));
                return;
            }
            worker.request(request, reply);
            return;
        }
        let _ = unlocked;
        let answer = match request {
            Request::GetPluginRuntime => Ok(Response::PluginRuntime {
                status: Box::new(idle_status(&self.settings)),
            }),
            _ => Err(unavailable(&self.settings)),
        };
        let _ = reply.send(answer);
    }

    /// The configuration was unlocked with `key`: open the plugin store
    /// with the key derived from it.
    pub(crate) fn opened(&self, key: &deckpilot_storage::DatabaseKey) {
        #[cfg(feature = "plugins")]
        if let Some(worker) = &self.worker {
            worker.open(key::plugin_store_key(key));
        }
        #[cfg(not(feature = "plugins"))]
        let _ = key;
    }

    /// The configuration was locked: stop every host and close the store.
    pub(crate) fn closed(&self) {
        #[cfg(feature = "plugins")]
        if let Some(worker) = &self.worker {
            worker.close();
        }
    }

    /// The configuration key changes from `old` to `new` (the agent
    /// switches it to a system-managed key): re-encrypt the plugin store
    /// with the key derived from `new`. Called while locked.
    pub(crate) fn rekey(
        &self,
        old: &deckpilot_storage::DatabaseKey,
        new: &deckpilot_storage::DatabaseKey,
    ) {
        #[cfg(feature = "plugins")]
        if let Some(worker) = &self.worker {
            worker.rekey(key::plugin_store_key(old), key::plugin_store_key(new));
        }
        #[cfg(not(feature = "plugins"))]
        let _ = (old, new);
    }

    /// Stops every host and the worker.
    pub(crate) fn stop(&mut self) {
        #[cfg(feature = "plugins")]
        if let Some(worker) = self.worker.take() {
            worker.stop();
        }
    }
}
