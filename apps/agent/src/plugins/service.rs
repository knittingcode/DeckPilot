//! The plugin service: one open plugin store and everything that acts on
//! it. Runs on the plugin worker thread (`worker.rs`), never on the
//! agent's service task, so a slow install or invocation does not stall
//! devices or IPC.
//!
//! Every operation follows the same order: registry verification
//! (signatures, freshness, revocations, compatibility), package
//! verification against the registry's pins (size, sha256, manifest hash,
//! permissions, artifact contract), then the store's crash-safe lifecycle
//! step, then the broker's grants. Plugin code runs only in a supervised
//! host process.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use deckpilot_core::alias::LegacyRouting;
use deckpilot_ipc::plugins::PluginEvent;
use deckpilot_ipc::plugins::{
    self as ipc, PluginInstallSource, PluginOperation, PluginRuntimeStatus, PluginStage,
    PluginUpdateOutcome, reason,
};
use deckpilot_ipc::{ErrorCode, IpcError, Request, Response, plugin_answers};
use deckpilot_plugin_abi::capability::{RunContext, RunTrigger};
use deckpilot_plugin_abi::protocol::{Enforcement, SandboxReport};
use deckpilot_plugin_abi::{ActionOutcome, ParamValue, Params};
use deckpilot_plugin_broker::{
    AuditDecision, AuditRecord, AuditSink, Broker, RateLimits, SystemClock,
};
use deckpilot_plugin_manifest::{
    ApiLevel, HostEnvironment, LEGACY_JS_ARTIFACT, MANIFEST_VERSION, Manifest, Platform, PluginId,
    RuntimeContract,
};
use deckpilot_plugin_package::{
    ExpectedPackage, Limits, MANIFEST, PackageError, read_archive, verify_package,
};
use deckpilot_plugin_registry::{InstallError, InstalledVerdict, Refresh, RegistryError};
use deckpilot_plugin_store::{
    Consent, GrantKey, InstallSource, ManagerOptions, PluginData, PluginManager, PluginRecord,
    PluginState, Retention, StoreError, StoreKey, UpdateOutcome, VerdictSource,
};
use deckpilot_plugin_supervisor::{
    AbortSignal, HostCommand, HostConfig, HostError, RestartPolicy, Supervisor, SupervisorError,
};
use semver::Version;
use tokio::sync::oneshot;
use tracing::{info, warn};

use super::bundled::{self, Bundle};
use super::convert;
use super::data;
use super::engine::CatalogPlugin;
use super::executor::resources::InvocationResources;
use super::executor::{DataAccess, Executor, Opener, OwnVariables, SharedData, Transport};
use super::lane::{Done, Ended, Job, Lane, LaneEnd, LaneError, Reply, StateDone};
use super::locales::Locales;
use super::migration::LegacySnapshot;
use super::official::{self, Official};
use super::registry::{Registry, download_error};
use super::variables::PublishedVariables;
use super::worker::{EngineFailure, EngineReply, Events, StateReply};
use super::{PluginSettings, platform, platform_problem};

#[path = "dependencies.rs"]
mod dependencies;

/// Who decides in the audit log.
const DECIDED_BY: &str = "user";
/// The shared code of an invocation whose parameters do not match the
/// action's declaration (refused by the agent, before the host).
const INVALID_PARAMETERS: &str = "invalid-parameters";
/// The plugin API level this agent provides: 1.1 adds `time.now` and
/// `random.bytes`, 1.2 key state (`docs/marketplace/plugin-key-state.md`),
/// 1.3 the manifest's action `controls`, `navigation` and `views`
/// (`docs/device-controls.md` §4), 1.4 a view's `context` (which toolbar
/// selectors the app shows with it), `dependencies` and actions'
/// `requiresDependencies` (`docs/marketplace/dependencies.md`), 1.5 the
/// `dialogs` permission and `dialog.open` (`docs/dialogs.md`), 1.6 the
/// manifest's `locales`, `defaultLocale` and `t:` texts and `locale.get` and
/// `locale.translate` (`docs/marketplace/sdk/localization.md`), 1.7 the
/// actions' `editorNavigation` (`docs/editor-navigation-and-simulation.md`),
/// 1.8 the actions' `title` (`docs/key-text.md`, "Plugin titles"), 1.9 the
/// privileged kinds of DeckPilot's own bundled plugins (`tools`,
/// `resources`, `projectContext`, `hostViews`, `openUrl.fromParams`,
/// `resource` parameters, the `workflows` and `projects` views, status
/// items and dashboard cards; `docs/marketplace/module-migration.md` §7)
/// and their capability operations: `resource` parameters and
/// `resource.resolve` are carried out, the others refused as not
/// implemented until their host side lands (WP-A4 to WP-A6).
/// A plugin whose manifest asks for a higher minor is `incompatible`.
const PLUGIN_API: ApiLevel = ApiLevel::new(1, 9);
/// Longest any invocation may take, whatever the manifest says.
const MAX_INVOCATION: Duration = Duration::from_secs(600);
/// Where the registry's verified state is kept (outside `plugins/`,
/// whose unknown entries the store's recovery sweeps away).
const REGISTRY_STATE_DIR: &str = "plugin-registry";
/// Without trusted root keys no registry is used (none built in, none
/// configured).
const NOT_CONFIGURED: &str = "no plugin registry is configured: no trusted root keys \
     (none built in, no --plugin-registry-roots); nothing is fetched or trusted without them";

/// Broker decisions go to the agent's log (operation, origin and
/// decision; never values).
#[derive(Debug)]
struct LogAudit;

impl AuditSink for LogAudit {
    fn record(&self, record: AuditRecord) {
        match record.decision {
            AuditDecision::Allowed => tracing::debug!(
                plugin = %record.plugin, op = record.op, target = ?record.target,
                "plugin capability allowed"
            ),
            AuditDecision::Denied(reason) => info!(
                plugin = %record.plugin, op = record.op, target = ?record.target, %reason,
                "plugin capability denied"
            ),
        }
    }
}

/// Without a registry nothing is revoked.
struct NoRevocations;

impl VerdictSource for NoRevocations {
    fn verdict(&self, _id: &PluginId, _version: &Version) -> InstalledVerdict {
        InstalledVerdict::Run
    }
}

fn error(code: ErrorCode, why: &str, message: impl Into<String>) -> IpcError {
    IpcError::new(code, message).with_reason(why)
}

fn store_error(error: &StoreError) -> IpcError {
    use StoreError as E;
    let message = error.to_string();
    match error {
        E::NotInstalled(_) => self::error(ErrorCode::NotFound, reason::NOT_INSTALLED, message),
        E::AlreadyInstalled { .. } => {
            self::error(ErrorCode::Failed, reason::ALREADY_INSTALLED, message)
        }
        E::ConsentRequired { .. } | E::BundledGrant { .. } => {
            self::error(ErrorCode::Failed, reason::CONSENT_REQUIRED, message)
        }
        E::SourceNotAllowed { .. } => {
            self::error(ErrorCode::InvalidRequest, reason::INVALID_PACKAGE, message)
        }
        E::UnknownGrant { .. } => self::error(
            ErrorCode::InvalidRequest,
            reason::UNKNOWN_PERMISSION,
            message,
        ),
        E::Revoked { .. } => self::error(ErrorCode::Failed, reason::REVOKED, message),
        E::Integrity { .. } => self::error(ErrorCode::Failed, reason::BROKEN, message),
        E::Package(package) => package_error(package),
        E::InvalidData(_) => self::error(ErrorCode::InvalidRequest, reason::INVALID_DATA, message),
        E::IllegalTransition { .. }
        | E::OperationInProgress { .. }
        | E::NoPreviousVersion(_)
        | E::NoStagedUpdate(_)
        | E::UpdatePending { .. }
        | E::VersionInstalled { .. }
        | E::NotNewer { .. }
        | E::Tombstoned { .. } => self::error(ErrorCode::Failed, reason::INVALID_STATE, message),
        E::RequiredBy { .. } => self::error(ErrorCode::Failed, reason::REQUIRED_BY, message),
        // The step's own reason, with the whole message.
        E::PlanFailed { source, .. } => {
            let inner = store_error(source);
            match inner.reason.clone() {
                Some(why) => self::error(inner.code, &why, message),
                None => IpcError::new(inner.code, message),
            }
        }
        _ => {
            warn!(%error, "plugin store error");
            IpcError::new(ErrorCode::Failed, message)
        }
    }
}

fn package_error(error: &PackageError) -> IpcError {
    let why = match error {
        PackageError::UnsupportedRuntime { .. } | PackageError::Module(_) => {
            reason::UNSUPPORTED_RUNTIME
        }
        _ => reason::INVALID_PACKAGE,
    };
    self::error(ErrorCode::Failed, why, error.to_string())
}

fn install_error(error: &InstallError) -> IpcError {
    let message = error.to_string();
    match error {
        InstallError::Revoked { .. } | InstallError::Yanked { .. } => {
            self::error(ErrorCode::Failed, reason::REVOKED, message)
        }
        InstallError::Incompatible { .. } | InstallError::AppTooOld { .. } => {
            self::error(ErrorCode::Failed, reason::INCOMPATIBLE, message)
        }
        InstallError::UnknownPlugin(_) | InstallError::UnknownVersion { .. } => {
            self::error(ErrorCode::NotFound, reason::UNKNOWN_PLUGIN, message)
        }
        InstallError::FirstPartyNotAuthorized { .. } => {
            self::error(ErrorCode::Failed, reason::INVALID_PACKAGE, message)
        }
        _ => self::error(ErrorCode::Failed, reason::REGISTRY, message),
    }
}

fn registry_error(error: &RegistryError) -> IpcError {
    self::error(
        ErrorCode::Failed,
        reason::REGISTRY,
        format!("the plugin registry failed verification: {error}"),
    )
}

fn invalid(message: impl Into<String>) -> IpcError {
    IpcError::new(ErrorCode::InvalidRequest, message)
}

fn plugin_id(text: &str) -> Result<PluginId, IpcError> {
    PluginId::parse(text).map_err(|e| invalid(format!("plugin: {e}")))
}

fn version(text: &str) -> Result<Version, IpcError> {
    Version::parse(text).map_err(|e| invalid(format!("version `{text}`: {e}")))
}

fn platform_gate() -> Result<(), IpcError> {
    match platform_problem() {
        Some(problem) => Err(error(
            ErrorCode::Unsupported,
            reason::PLATFORM_UNSUPPORTED,
            problem,
        )),
        None => Ok(()),
    }
}

/// The host executable: configured, or next to the agent.
fn host_program(settings: &PluginSettings) -> PathBuf {
    settings.host.clone().unwrap_or_else(|| {
        let name = format!("deckpilot-plugin-host{}", std::env::consts::EXE_SUFFIX);
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(&name)))
            .unwrap_or_else(|| PathBuf::from(name))
    })
}

fn host_environment() -> Option<HostEnvironment> {
    Some(HostEnvironment {
        app_version: Version::parse(crate::service::VERSION).ok()?,
        plugin_api: PLUGIN_API,
        platform: Platform::current()?,
    })
}

/// Test failpoints (feature `test-hooks` only): `DECKPILOT_TEST_PLUGIN_CRASH`
/// = `<operation>:<failpoint>` (e.g. `install:files_unpacked`) ends the
/// agent process at that point of the store's lifecycle, as a power cut
/// would, so recovery can be tested with a real restart.
#[cfg(feature = "test-hooks")]
mod crash {
    use deckpilot_plugin_store::{Failpoint, Failpoints, Operation};

    pub const ENV: &str = "DECKPILOT_TEST_PLUGIN_CRASH";

    #[derive(Debug)]
    pub struct ExitAt(Operation, Failpoint);

    impl Failpoints for ExitAt {
        fn should_crash(&self, op: Operation, point: Failpoint) -> bool {
            if op == self.0 && point == self.1 {
                tracing::warn!(?op, ?point, "test failpoint: the agent exits now");
                std::process::exit(86);
            }
            false
        }
    }

    pub fn from_env() -> Option<ExitAt> {
        let text = std::env::var(ENV).ok()?;
        let (op, point) = text.split_once(':')?;
        let op = Operation::ALL.into_iter().find(|o| o.as_str() == op)?;
        let point = match point {
            "journal_written" => Failpoint::JournalWritten,
            "files_unpacked" => Failpoint::FilesUnpacked,
            "step_recorded" => Failpoint::StepRecorded,
            "committed" => Failpoint::Committed,
            _ => return None,
        };
        Some(ExitAt(op, point))
    }
}

fn manager_options() -> ManagerOptions {
    #[cfg(feature = "test-hooks")]
    if let Some(exit) = crash::from_env() {
        return ManagerOptions {
            failpoints: Arc::new(exit),
            ..ManagerOptions::default()
        };
    }
    ManagerOptions::default()
}

pub(super) struct PluginService {
    manager: PluginManager,
    registry: Option<Registry>,
    /// Why there is no registry, or why it last failed to verify.
    registry_problem: Option<String>,
    broker: Broker,
    /// One lane (thread and supervisor) per plugin whose host runs or ran.
    lanes: BTreeMap<String, Lane>,
    next_lane: u64,
    /// Tells the worker that a lane ended by itself.
    lane_ended: Ended,
    host: PathBuf,
    events: Events,
    transport: Transport,
    /// Where allowed `url.open` requests go.
    opener: Opener,
    /// The capability executors' connection to plugins.db (storage,
    /// secrets, settings); emptied when the service shuts down.
    data: SharedData,
    /// What the plugins published for their own variables.
    values: PublishedVariables,
    /// Host failures per plugin since the agent started.
    health: HealthLog,
    /// The first-party plugins of the application bundle and the pins
    /// compiled into the agent (`bundled.rs`).
    bundle: Bundle,
    /// Official releases from a registry: publisher keys, revocations,
    /// rollback floors and their audit log (`official.rs`).
    official: Official,
    /// What `deckpilot.db` said about the modules that can migrate at the
    /// agent's last module load (the user's decisions need it); `None`
    /// until then and whenever no alias exists. Its secrets never leave
    /// this process and are never logged.
    snapshot: Option<LegacySnapshot>,
    /// The agent's built-in modules, for dependencies on them (sent by
    /// the agent after every module load).
    builtins: BTreeMap<String, deckpilot_plugin_store::deps::BuiltinModule>,
    /// The verified packages of the last install plan.
    plan_cache: Option<dependencies::PlanCache>,
    /// The app's language and the plugins' verified translations (plugin
    /// API 1.6).
    locales: Locales,
    /// Decoded visual asset images by the sha256 of their file, so the
    /// catalog decodes each file once.
    asset_images: std::sync::Mutex<
        std::collections::HashMap<
            deckpilot_plugin_manifest::Sha256Digest,
            deckpilot_render::LibraryAsset,
        >,
    >,
}

/// What the agent saw of each plugin's host since it started
/// ([`ipc::PluginSummary::health`]); in memory only. Written on the lane
/// threads when an invocation fails, read when a plugin is listed.
#[derive(Debug, Clone, Default)]
struct HealthLog(Arc<Mutex<BTreeMap<String, ipc::PluginHealth>>>);

impl HealthLog {
    /// Records a failed invocation of `plugin`. Refusals that say nothing
    /// about the host (busy, stopped, cancelled before it started) are not
    /// recorded; a crash or a timeout also counts as a crash.
    fn record(&self, plugin: &str, error: &LaneError, at: i64) {
        let crashed = match error {
            LaneError::Busy
            | LaneError::Stopped
            | LaneError::Aborted
            | LaneError::Supervisor(SupervisorError::Host(HostError::Aborted { .. })) => return,
            LaneError::Supervisor(SupervisorError::Host(
                HostError::Crashed { .. } | HostError::Timeout { .. },
            )) => true,
            LaneError::Supervisor(_) | LaneError::Refused(_) | LaneError::Panicked => false,
        };
        let Ok(mut log) = self.0.lock() else {
            return;
        };
        let health = log.entry(plugin.to_owned()).or_default();
        if crashed {
            health.crashes = health.crashes.saturating_add(1);
        }
        health.last_error = Some(lane_error(error).message);
        health.last_error_at = Some(at);
    }

    fn get(&self, plugin: &str) -> Option<ipc::PluginHealth> {
        self.0.lock().ok()?.get(plugin).cloned()
    }

    fn forget(&self, plugin: &str) {
        if let Ok(mut log) = self.0.lock() {
            log.remove(plugin);
        }
    }

    /// `done`, after recording a failure of `plugin`.
    fn watch(&self, plugin: String, done: Done) -> Done {
        let log = self.clone();
        Box::new(move |result| {
            if let Err(e) = &result {
                log.record(&plugin, e, Utc::now().timestamp());
            }
            done(result);
        })
    }
}

impl std::fmt::Debug for PluginService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginService")
            .field("registry", &self.registry)
            .field("running", &self.lanes.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// What an invocation needs once it is allowed to run.
struct Prepared {
    timeout: Duration,
    outputs: BTreeSet<String>,
    executor: Executor,
    /// The action's parameter schema, for validating the parameters.
    definition: deckpilot_core::ActionType,
    /// The declared kinds of each `resource` parameter (plugin API 1.9).
    resource_kinds: BTreeMap<String, Vec<deckpilot_plugin_abi::privileged::ResourceKind>>,
    /// The active version, for the audit.
    version: String,
}

/// Why the plugin store could not be opened.
pub(super) fn open_problem(error: &StoreError) -> String {
    match error {
        StoreError::InvalidKey => "plugins.db cannot be opened with the key derived from the \
             configuration key (it was written under another configuration key, e.g. before a \
             password change made while the agent was locked); plugins stay unavailable and \
             plugins.db is left untouched"
            .to_owned(),
        other => format!("the plugin store cannot be opened: {other}"),
    }
}

impl PluginService {
    pub fn open(
        settings: &PluginSettings,
        data_dir: &Path,
        key: &StoreKey,
        events: Events,
        lane_ended: Ended,
        values: PublishedVariables,
    ) -> Result<Self, StoreError> {
        let mut manager = PluginManager::open(data_dir, key, manager_options())?;
        let report = manager.recover()?;
        // Offline, before anything reads the installed plugins: the
        // bundled first-party plugins, checked against the pins compiled
        // into the agent.
        let bundle = Bundle::locate(settings);
        bundled::seed(&mut manager, &bundle);
        let data = PluginData::open(
            &manager.layout().database(),
            key,
            deckpilot_plugin_store::DEFAULT_BUSY_TIMEOUT,
        )?;
        if !report.is_clean() {
            info!(?report, "plugin store recovered an interrupted operation");
            let _ = events.plugins.send(PluginEvent::Progress {
                plugin: String::new(),
                operation: PluginOperation::Recover,
                stage: PluginStage::Finished,
                message: Some(format!("{report:?}")),
            });
        }
        let (registry, registry_problem) =
            match Registry::from_settings(settings, &data_dir.join(REGISTRY_STATE_DIR)) {
                Ok(Some(registry)) => (Some(registry), None),
                Ok(None) => (None, Some(NOT_CONFIGURED.to_owned())),
                Err(problem) => (None, Some(problem)),
            };
        let mut service = Self {
            manager,
            registry,
            registry_problem,
            broker: Broker::new(
                Arc::new(LogAudit),
                Arc::new(SystemClock::default()),
                RateLimits::default(),
            ),
            lanes: BTreeMap::new(),
            next_lane: 0,
            lane_ended,
            host: host_program(settings),
            events,
            transport: Transport::from_env(),
            opener: Opener::from_env(),
            data: Arc::new(Mutex::new(Some(data))),
            values,
            health: HealthLog::default(),
            bundle,
            official: Official::open(&data_dir.join(official::STATE_DIR)),
            snapshot: None,
            builtins: BTreeMap::new(),
            plan_cache: None,
            locales: Locales::default(),
            asset_images: std::sync::Mutex::default(),
        };
        if service.registry.is_some() {
            let _ = service.refresh();
        }
        // Also offline and without a registry: the official revocations
        // compiled into this agent.
        service.apply_verdicts();
        for record in service.manager.store().plugins()? {
            service.sync_grants(&record.id);
        }
        Ok(service)
    }

    /// Stops every host; every published value is dropped.
    pub fn shutdown(&mut self) {
        for (_, lane) in std::mem::take(&mut self.lanes) {
            lane.stop();
        }
        // Closes the executors' connection, so nothing keeps plugins.db
        // open after a lock (or a rekey).
        if let Ok(mut data) = self.data.lock() {
            *data = None;
        }
        for (plugin, ids) in self.values.forget_all() {
            self.announce_dropped(&plugin, &ids);
        }
    }

    /// Tells clients that a plugin's values are gone (ids only).
    fn announce_dropped(&self, plugin: &str, ids: &[String]) {
        let Ok(module) = deckpilot_core::ModuleId::new(plugin) else {
            return;
        };
        let namespace = deckpilot_core::variables::module_namespace(&module);
        if !ids.is_empty() {
            let _ = self
                .events
                .agent
                .send(deckpilot_ipc::AgentEvent::SharedVariablesChanged {
                    keys: ids.iter().map(|id| format!("{namespace}.{id}")).collect(),
                });
        }
    }

    /// What `deckpilot.db` says about the modules that can migrate, after
    /// every module load; with `first` (the first snapshot after the store
    /// opened) the migration pass runs (`migration.rs`). The plugins it
    /// installed, enabled or granted get their grants in the broker and an
    /// event, and every migration that moved gets one.
    pub fn migrate(&mut self, snapshot: LegacySnapshot, first: bool) {
        if first {
            let before = self.journal();
            let touched = super::migration::start(&mut self.manager, &self.bundle, &snapshot);
            self.after_migration(&touched, &before, None);
        }
        // The host-held tool settings the migration keeps (`tool.run`).
        super::tool_settings::held(&self.manager);
        self.snapshot = Some(snapshot);
    }

    /// The migration journal now (for change events).
    fn journal(&self) -> Vec<deckpilot_plugin_store::ModuleMigration> {
        self.manager.store().module_migrations().unwrap_or_default()
    }

    /// After migration steps: the broker syncs the touched plugins, which
    /// get an event, and every migration whose record changed since
    /// `before` (and `decided`, whatever happened) gets one.
    fn after_migration(
        &mut self,
        touched: &BTreeSet<PluginId>,
        before: &[deckpilot_plugin_store::ModuleMigration],
        decided: Option<&str>,
    ) {
        for id in touched {
            self.sync_grants(id);
            self.changed(id);
        }
        for record in self.journal() {
            let moved = !before.contains(&record) || decided == Some(record.module_id.as_str());
            if moved && !touched.contains(&record.plugin_id) {
                let _ = self
                    .events
                    .plugins
                    .send(super::migration::event_of(&record));
            }
        }
    }

    /// `decideModuleMigration` and `retryModuleMigration`.
    fn decide_migration(
        &mut self,
        decision: &super::migration::Decision,
    ) -> Result<Response, IpcError> {
        use super::migration::Refusal;
        let module = decision.module().to_owned();
        let snapshot = self.snapshot.as_ref().ok_or_else(|| {
            error(
                ErrorCode::NotFound,
                reason::UNKNOWN_MIGRATION,
                format!("no module `{module}` can migrate here"),
            )
        })?;
        let before = self.journal();
        let touched = super::migration::decide(&mut self.manager, &self.bundle, snapshot, decision)
            .map_err(|refusal| match &refusal {
                Refusal::Unknown(why) => {
                    error(ErrorCode::NotFound, reason::UNKNOWN_MIGRATION, why.clone())
                }
                // A store refusal is a state the request cannot change.
                Refusal::InvalidState(why) | Refusal::Failed(why) => {
                    error(ErrorCode::Failed, reason::INVALID_STATE, why.clone())
                }
                Refusal::UnknownItem(why) => error(
                    ErrorCode::InvalidRequest,
                    reason::UNKNOWN_PERMISSION,
                    why.clone(),
                ),
            })?;
        self.after_migration(&touched, &before, Some(&module));
        self.migration_info(&module)
    }

    fn migration_info(&self, module: &str) -> Result<Response, IpcError> {
        super::migration::info(&self.manager, &self.bundle, self.snapshot.as_ref(), module)
            .map(|migration| Response::ModuleMigration {
                migration: Box::new(migration),
            })
            .ok_or_else(|| {
                error(
                    ErrorCode::NotFound,
                    reason::UNKNOWN_MIGRATION,
                    format!("no migration of `{module}` is known"),
                )
            })
    }

    /// Where the legacy action types of migrated modules go now.
    pub fn legacy_routing(&self) -> LegacyRouting {
        super::migration::routing(&self.manager, &self.bundle)
    }

    /// The installed plugins and whether their actions can run, for the
    /// agent's module registry. Unreadable records are left out.
    pub fn catalog(&self) -> Vec<CatalogPlugin> {
        let store = self.manager.store();
        let Ok(records) = store.plugins() else {
            return Vec::new();
        };
        // Plugins that were installed once and are not now (design §2.3
        // item 4): their actions stay known, unavailable, with the
        // plugin's name.
        let gone = store
            .known()
            .unwrap_or_default()
            .into_iter()
            .filter(|known| !records.iter().any(|r| r.id == known.id))
            .map(|known| CatalogPlugin {
                id: known.id.to_string(),
                // A translated name (`t:plugin.name`) cannot be shown once
                // its locales are gone with the plugin: its id instead.
                name: if known.display_name.is_empty() || known.display_name.starts_with("t:") {
                    known.id.to_string()
                } else {
                    known.display_name
                },
                default_names: BTreeMap::new(),
                version: known.last_version,
                description: String::new(),
                actions: known.actions,
                // `plugin_known` keeps no variable declarations: references
                // to them stay unavailable as before.
                variables: Vec::new(),
                unavailable: Some("the plugin is not installed".to_owned()),
                action_unavailable: BTreeMap::new(),
                requires: Vec::new(),
                visual_assets: Vec::new(),
                project_context: false,
                resources: false,
                wrap_around: None,
            });
        let graph = self.graph();
        self.locales
            .retain(&records.iter().map(|r| r.id.to_string()).collect());
        records
            .iter()
            .filter_map(|record| {
                let active = record.active()?;
                let dir = self.manager.layout().resolve(&active.install_dir);
                // What the app and the keys show, in the app's language.
                let shown = self.locales.manifest(record.id.as_str(), active, &dir);
                let manifest = &shown;
                let grants = store.grants(&record.id).unwrap_or_default();
                let (missing, mut action_unmet) = self.unmet(&active.manifest, &graph);
                // An optional permission not granted (plugin API 1.9):
                // only the actions that need it wait.
                for (action, why) in convert::unmet_permissions(&active.manifest, &grants) {
                    action_unmet.entry(action).or_insert(why);
                }
                let default_names = active
                    .manifest
                    .contributes
                    .actions()
                    .iter()
                    .map(|a| {
                        let names =
                            self.locales
                                .action_names(record.id.as_str(), active, &dir, &a.id);
                        (a.id.clone(), names)
                    })
                    .filter(|(_, names)| !names.is_empty())
                    .collect();
                let project_context = grants.iter().any(|g| {
                    g.key.kind == deckpilot_plugin_manifest::PermissionKind::ProjectContext
                        && g.key.key.is_empty()
                        && g.status == deckpilot_plugin_store::GrantStatus::Granted
                });
                let wrap_around = project_context.then(|| {
                    store
                        .settings(&record.id)
                        .ok()
                        .and_then(|settings| settings.get("wrapAround")?.as_bool())
                        .unwrap_or(true)
                });
                Some(CatalogPlugin {
                    id: record.id.to_string(),
                    name: manifest.name.clone(),
                    version: manifest.version.to_string(),
                    description: manifest.description.clone(),
                    actions: manifest.contributes.actions().to_vec(),
                    variables: manifest.contributes.variables().to_vec(),
                    unavailable: convert::unavailable_with(record, &grants).or(missing),
                    action_unavailable: action_unmet,
                    requires: manifest
                        .required_dependencies()
                        .map(|d| d.id.to_string())
                        .collect(),
                    default_names,
                    visual_assets: self.drawable_assets(record, &dir),
                    project_context,
                    wrap_around,
                    resources: grants.iter().any(|g| {
                        g.key.kind == deckpilot_plugin_manifest::PermissionKind::Resources
                            && g.key.key.is_empty()
                            && g.status == deckpilot_plugin_store::GrantStatus::Granted
                    }),
                })
            })
            .chain(gone)
            .collect()
    }

    /// The visual assets keys may draw of `record` (installed in `dir`):
    /// only while the plugin is enabled, each file checked against the
    /// integrity record and decoded with limits (a file that fails is left
    /// out; its keys draw their fallback). No plugin code runs for it.
    fn drawable_assets(
        &self,
        record: &deckpilot_plugin_store::PluginRecord,
        dir: &std::path::Path,
    ) -> Vec<(deckpilot_core::AssetId, deckpilot_render::LibraryAsset)> {
        use deckpilot_plugin_manifest::VisualBackgroundKind;
        if record.state != PluginState::Enabled {
            return Vec::new();
        }
        let Some(active) = record.active() else {
            return Vec::new();
        };
        let plugin = record.id.to_string();
        let assets = active.manifest.contributes.visual_assets();
        let image = |asset: &str| -> Option<deckpilot_render::LibraryAsset> {
            let (_, bytes) = convert::visual_asset_file(dir, record, asset)?;
            let digest = convert::sha256_of(&bytes);
            let mut cache = self.asset_images.lock().ok()?;
            if let Some(image) = cache.get(&digest) {
                return Some(image.clone());
            }
            let decoded = match deckpilot_render::icons::decode_asset(&bytes) {
                Ok(decoded) => deckpilot_render::LibraryAsset::Image(Arc::new(decoded)),
                Err(error) => {
                    warn!(%plugin, asset, %error, "a visual asset cannot be decoded");
                    return None;
                }
            };
            cache.insert(digest, decoded.clone());
            Some(decoded)
        };
        let icons = assets
            .icons()
            .iter()
            .filter_map(|icon| Some((icon.id.as_str(), image(&icon.id)?)));
        let backgrounds = assets.backgrounds().iter().filter_map(|background| {
            let drawn = match background.kind {
                VisualBackgroundKind::Image => image(&background.id)?,
                VisualBackgroundKind::Gradient => {
                    let gradient = deckpilot_core::Gradient {
                        colors: background
                            .colors
                            .iter()
                            .flatten()
                            .map(|c| deckpilot_core::Color::parse(c).ok())
                            .collect::<Option<_>>()?,
                        angle: background.angle.unwrap_or(180),
                    };
                    gradient.check().ok()?;
                    deckpilot_render::LibraryAsset::Gradient(gradient)
                }
            };
            Some((background.id.as_str(), drawn))
        });
        icons
            .chain(backgrounds)
            .filter_map(|(id, asset)| {
                Some((deckpilot_core::AssetId::of_plugin(&plugin, id).ok()?, asset))
            })
            .collect()
    }

    /// The app's language changed (`setLocale`, protocol 13): the
    /// plugins whose manifests name translations are announced as changed,
    /// so the app reads their texts again, and the worker publishes the
    /// catalog in the new language (the Action Library and the keys).
    /// Whether it changed.
    pub fn set_locale(&mut self, locale: &str) -> bool {
        if !self.locales.set_locale(locale) {
            return false;
        }
        info!(locale = %self.locales.tag(), "plugin texts follow the app's language");
        let Ok(records) = self.manager.store().plugins() else {
            return true;
        };
        for record in records.iter().filter(|r| {
            r.versions
                .iter()
                .any(|v| v.manifest.reads_translation_keys())
        }) {
            let _ = self.events.plugins.send(PluginEvent::Changed {
                plugin: record.id.to_string(),
                state: Some(convert::state(record.state)),
            });
        }
        true
    }

    /// `record` as the app shows it: its manifests' `t:` texts in the
    /// app's language.
    fn shown(&self, record: &PluginRecord) -> PluginRecord {
        let layout = self.manager.layout();
        self.locales.localized(record, &|dir| layout.resolve(dir))
    }

    pub fn status(&self) -> PluginRuntimeStatus {
        PluginRuntimeStatus {
            available: true,
            built: true,
            enabled: true,
            execution_supported: super::execution_supported(),
            platform: platform().to_owned(),
            // Both, when both apply: a platform that cannot run plugins
            // must not hide that the registry was refused.
            reason: match (platform_problem(), self.registry_problem.clone()) {
                (Some(platform), Some(registry)) => Some(format!("{registry}; {platform}")),
                (platform, registry) => registry.or(platform),
            },
            registry: self.registry.as_ref().map(Registry::status),
            features: super::features(),
        }
    }

    // ------------------------------------------------------------- helpers

    fn progress(&self, plugin: &str, operation: PluginOperation, stage: PluginStage) {
        let _ = self.events.plugins.send(PluginEvent::Progress {
            plugin: plugin.to_owned(),
            operation,
            stage,
            message: None,
        });
    }

    fn failed(&self, plugin: &str, operation: PluginOperation, error: &IpcError) {
        let _ = self.events.plugins.send(PluginEvent::Progress {
            plugin: plugin.to_owned(),
            operation,
            stage: PluginStage::Failed,
            message: Some(error.message.clone()),
        });
    }

    fn changed(&self, id: &PluginId) {
        let state = self
            .manager
            .store()
            .plugin(id)
            .ok()
            .flatten()
            .map(|r| convert::state(r.state));
        let _ = self.events.plugins.send(PluginEvent::Changed {
            plugin: id.to_string(),
            state,
        });
        // A plugin that replaces a module: its migration's route and
        // consent items may have changed with it (disabled, uninstalled,
        // a grant).
        for record in self.journal() {
            if &record.plugin_id == id {
                let _ = self
                    .events
                    .plugins
                    .send(super::migration::event_of(&record));
            }
        }
    }

    fn record(&self, id: &PluginId) -> Result<PluginRecord, IpcError> {
        self.manager
            .store()
            .plugin(id)
            .map_err(|e| store_error(&e))?
            .ok_or_else(|| store_error(&StoreError::NotInstalled(id.clone())))
    }

    fn details(&self, id: &PluginId) -> Result<Response, IpcError> {
        self.details_as(id, false)
    }

    /// `getPlugin`, or with `paged` `getPluginPaged` (no actions, no icon).
    fn details_as(&self, id: &PluginId, paged: bool) -> Result<Response, IpcError> {
        let record = self.record(id)?;
        let store = self.manager.store();
        let grants = store.grants(id).map_err(|e| store_error(&e))?;
        let audit = store.audit(id).map_err(|e| store_error(&e))?;
        let pending = self
            .manager
            .pending_update(id)
            .map_err(|e| store_error(&e))?;
        let record = self.shown(&record);
        let mut details = convert::details(&record, &grants, &audit, pending);
        details.summary.health = self.health.get(id.as_str());
        if paged {
            plugin_answers::paged_details(&mut details);
        } else {
            details.icon = record.active().and_then(|v| {
                convert::icon(&self.manager.layout().resolve(&v.install_dir), &record)
            });
        }
        details.storage = self.storage_usage(id);
        self.add_dependencies(&mut details, id);
        Ok(Response::Plugin {
            plugin: Box::new(details),
        })
    }

    /// `listPlugins`, or with `paged` `listPluginsPaged` (no actions).
    fn list(&self, paged: bool) -> Result<Response, IpcError> {
        let store = self.manager.store();
        let graph = self.graph();
        let auto = store.dependency_installs().unwrap_or_default();
        let plugins = store.plugins().map_err(|e| store_error(&e))?;
        Ok(Response::Plugins {
            plugins: plugins
                .iter()
                .map(|record| {
                    let shown = self.shown(record);
                    let mut summary =
                        convert::summary(&shown, &store.grants(&record.id).unwrap_or_default());
                    summary.health = self.health.get(record.id.as_str());
                    self.apply_unmet(&mut summary, record, &graph);
                    summary.installed_for = auto.get(&record.id).cloned();
                    if paged {
                        plugin_answers::paged_summary(&mut summary);
                    }
                    summary
                })
                .collect(),
        })
    }

    /// `getPluginActions`: the actions from `from` on that fit one answer.
    fn actions_page(&self, id: &PluginId, from: u32) -> Result<Response, IpcError> {
        let record = self.record(id)?;
        let grants = self
            .manager
            .store()
            .grants(id)
            .map_err(|e| store_error(&e))?;
        let mut entries = convert::action_entries(&self.shown(&record), &grants);
        if let Some(manifest) = record.active().map(|v| &v.manifest) {
            let (missing, unmet) = self.unmet(manifest, &self.graph());
            for entry in &mut entries {
                let action = &mut entry.action;
                if let (true, Some(why)) = (
                    action.available,
                    missing
                        .clone()
                        .or_else(|| unmet.get(&action.action).cloned()),
                ) {
                    action.available = false;
                    action.unavailable_reason = Some(why);
                }
            }
        }
        let total = u32::try_from(entries.len()).unwrap_or(u32::MAX);
        let (actions, next) = plugin_answers::page_actions(entries, from);
        for entry in actions.iter().filter(|e| e.too_large) {
            warn!(
                plugin = %id,
                action = %entry.action.action,
                "an action's definition does not fit one IPC message; the editors do not offer it"
            );
        }
        Ok(Response::PluginActions {
            plugin: id.to_string(),
            version: record
                .active()
                .map(|v| v.version.to_string())
                .unwrap_or_default(),
            total,
            actions,
            next,
        })
    }

    /// `getPluginIcon`: the chunk of the icon at `offset`.
    fn icon_chunk(
        &self,
        id: &PluginId,
        offset: u64,
        navigation: Option<&ipc::NavigationIconRef>,
        asset: Option<&str>,
    ) -> Result<Response, IpcError> {
        let record = self.record(id)?;
        let icon = record.active().and_then(|v| {
            let dir = self.manager.layout().resolve(&v.install_dir);
            match (navigation, asset) {
                (_, Some(asset)) => convert::visual_asset_chunk(&dir, &record, asset, offset),
                (None, None) => convert::icon_chunk(&dir, &record, offset),
                (Some(navigation), None) => {
                    convert::navigation_icon_chunk(&dir, &record, navigation, offset)
                }
            }
        });
        Ok(Response::PluginIcon {
            plugin: id.to_string(),
            icon,
        })
    }

    /// What `id`'s storage holds, and the quota its current grants give it
    /// (as the broker computes it for every write).
    fn storage_usage(&self, id: &PluginId) -> Option<ipc::PluginStorageUsage> {
        let store = self.manager.store();
        let usage = store.storage_usage(id).ok()?;
        let effective = store.effective_grants(id).unwrap_or_default();
        let quota = deckpilot_plugin_broker::Grants::derive(&convert::approved(&effective).0)
            .map(|g| g.storage_quota())
            .unwrap_or_default();
        Some(ipc::PluginStorageUsage {
            used_bytes: usage.bytes,
            keys: usage.keys,
            quota_bytes: quota.bytes,
            max_keys: quota.keys,
        })
    }

    fn registry(&self) -> Result<&Registry, IpcError> {
        self.registry.as_ref().ok_or_else(|| {
            error(
                ErrorCode::Failed,
                reason::REGISTRY,
                self.registry_problem
                    .clone()
                    .unwrap_or_else(|| NOT_CONFIGURED.into()),
            )
        })
    }

    /// Re-reads and verifies the registry. A refused document changes
    /// nothing stored and fails the operation that needed it.
    fn refresh(&mut self) -> Result<Refresh, IpcError> {
        let Some(registry) = self.registry.as_mut() else {
            return Err(self.registry().unwrap_err());
        };
        match registry.refresh() {
            Ok(refresh) => {
                self.registry_problem = None;
                // A new index may revoke an installed version.
                if matches!(refresh, Refresh::Updated { .. }) {
                    self.apply_verdicts();
                }
                Ok(refresh)
            }
            Err(e) => {
                warn!(error = %e, "the plugin registry was refused");
                self.registry_problem = Some(format!("the plugin registry was refused: {e}"));
                Err(registry_error(&e))
            }
        }
    }

    /// Applies the stored revocations to installed plugins (also offline).
    fn apply_verdicts(&mut self) {
        let records = self.manager.store().plugins().unwrap_or_default();
        let revoked = self.official.revoked_installed(&records);
        let Some(registry) = self.registry.as_ref() else {
            // No registry: only the official revocations, which quarantine
            // (nothing is lifted without the registry's verdicts).
            for ((id, version), why) in &revoked {
                let quarantine = records.iter().any(|r| {
                    r.id == *id
                        && r.state != PluginState::Quarantined
                        && r.state.is_stable()
                        && r.active().is_some_and(|a| a.version == *version)
                });
                if quarantine {
                    match self.manager.quarantine(id, &format!("revoked: {why}")) {
                        Ok(()) => {
                            info!(plugin = %id, %version, %why, "official revocation applied");
                            self.stop_host(id);
                            self.sync_grants(id);
                            self.changed(id);
                        }
                        Err(e) => {
                            warn!(plugin = %id, error = %e, "official revocation not applied");
                        }
                    }
                }
            }
            return;
        };
        let verdicts = official::Verdicts {
            official: &revoked,
            registry: registry.client(),
        };
        match self.manager.apply_verdicts(&verdicts) {
            Ok(changes) => {
                for change in changes {
                    info!(?change, "registry verdict applied");
                    let id = match &change {
                        deckpilot_plugin_store::VerdictChange::Quarantined { id, .. }
                        | deckpilot_plugin_store::VerdictChange::Released { id }
                        | deckpilot_plugin_store::VerdictChange::StagedDropped { id, .. } => {
                            id.clone()
                        }
                    };
                    self.stop_host(&id);
                    self.sync_grants(&id);
                    self.changed(&id);
                }
            }
            Err(e) => warn!(error = %e, "registry verdicts could not be applied"),
        }
    }

    /// The version to install: `requested`, or the latest stable one.
    fn pick_version(&self, id: &PluginId, requested: Option<&str>) -> Result<Version, IpcError> {
        if let Some(text) = requested {
            return version(text);
        }
        let registry = self.registry()?;
        let index = registry.client().index().ok_or_else(|| {
            error(
                ErrorCode::Failed,
                reason::REGISTRY,
                "the plugin registry has never been verified",
            )
        })?;
        index
            .plugin(id)
            .ok_or_else(|| {
                error(
                    ErrorCode::NotFound,
                    reason::UNKNOWN_PLUGIN,
                    format!("{id} is not in the registry"),
                )
            })?
            .latest
            .stable
            .clone()
            .ok_or_else(|| {
                error(
                    ErrorCode::NotFound,
                    reason::UNKNOWN_PLUGIN,
                    format!("{id} has no stable version"),
                )
            })
    }

    /// Registry pins for `id` `version` and the package bytes they cover
    /// (downloaded; nothing is written before they verified).
    fn fetch(
        &self,
        id: &PluginId,
        version: &Version,
    ) -> Result<(ExpectedPackage, Vec<u8>), IpcError> {
        let target = self.pins(id, version)?;
        let registry = self.registry()?;
        let bytes = registry
            .package(&target.url, &target.expected.package)
            .map_err(|e| download_error(&e))?;
        // An official plugin: a first-party release signed by the
        // publisher keys compiled into this agent, on top of the registry's
        // pins (`official.rs`). Nothing is installed before it verified.
        if deckpilot_plugin_store::is_first_party(id) {
            let signature = match registry.signature(&target.url) {
                Ok(signature) => signature,
                Err(e) => {
                    let refused = download_error(&e);
                    self.official.refused_before_verification(
                        id,
                        Some(version),
                        Some(&target.expected.package),
                        &format!("the publisher signature: {}", refused.message),
                        Utc::now(),
                    );
                    return Err(refused);
                }
            };
            let host = host_environment();
            self.official
                .verify(&official::Delivery {
                    expected: &target.expected,
                    package: &bytes,
                    signatures: std::slice::from_ref(&signature),
                    pin: self.bundle.pins.get(id),
                    host: host.as_ref(),
                    now: Utc::now(),
                })
                .map_err(|refusal| refusal.to_ipc(id))?;
        }
        Ok((target.expected, bytes))
    }

    /// What the registry pins for `id` `version`, without the package.
    fn pins(
        &self,
        id: &PluginId,
        version: &Version,
    ) -> Result<deckpilot_plugin_registry::InstallTarget, IpcError> {
        // An official plugin (`deckpilot.*`, the bundled ones included)
        // comes from a registry only as a first-party release signed by the
        // publisher keys compiled into this agent (`official.rs`); without
        // them (none are provisioned yet) it is refused before any download
        // and a bundled one comes only from the application bundle.
        if deckpilot_plugin_store::is_first_party(id)
            && let Err(why) = self.official.ready()
        {
            self.official.refused_before_verification(
                id,
                Some(version),
                None,
                &format!("refused before any download: {why}"),
                Utc::now(),
            );
            let what = if deckpilot_plugin_bundled::is_bundled_id(id, &self.bundle.pins) {
                "is bundled with DeckPilot and comes only from the application bundle"
            } else {
                "is an official DeckPilot plugin and is never installed from a registry"
            };
            return Err(error(
                ErrorCode::InvalidRequest,
                reason::INVALID_PACKAGE,
                format!(
                    "{id} {what} unless the release is signed by the DeckPilot publisher: {why}"
                ),
            ));
        }
        // Ids of pre-schema-3 action types: never assigned to any plugin.
        if deckpilot_core::RESERVED_MODULE_IDS.contains(&id.as_str()) {
            return Err(error(
                ErrorCode::InvalidRequest,
                reason::INVALID_PACKAGE,
                format!("{id} is reserved and never assigned to a plugin"),
            ));
        }
        let registry = self.registry()?;
        // Revocations first: a revoked plugin may no longer be listed at
        // all, and must be refused as revoked, not as unknown.
        match registry.client().installed_verdict(id, version) {
            InstalledVerdict::Disable { reason: why, .. } => {
                return Err(error(
                    ErrorCode::Failed,
                    reason::REVOKED,
                    format!("{id} {version} was revoked by the registry: {why}"),
                ));
            }
            InstalledVerdict::RunWithNotice { reason: why } => {
                return Err(error(
                    ErrorCode::Failed,
                    reason::REVOKED,
                    format!("{id} {version} was withdrawn by the registry: {why}"),
                ));
            }
            InstalledVerdict::Run => {}
        }
        if registry.client().detail_ref(id).is_none() {
            return Err(error(
                ErrorCode::NotFound,
                reason::UNKNOWN_PLUGIN,
                format!("{id} is not in the registry"),
            ));
        }
        let detail = registry
            .detail_bytes(id)
            .map_err(|m| error(ErrorCode::Failed, reason::REGISTRY, m))?;
        let target = registry
            .client()
            .resolve_install(
                id,
                version,
                &detail,
                host_environment().as_ref(),
                Utc::now(),
            )
            .map_err(|e| install_error(&e))?;
        Ok(target)
    }

    fn install_source(&self) -> InstallSource {
        InstallSource::Registry {
            registry: self
                .registry
                .as_ref()
                .map_or_else(|| "registry".to_owned(), |r| r.name.clone()),
        }
    }

    fn consent(consent: &[String]) -> Result<Consent, IpcError> {
        Ok(Consent {
            keys: convert::keys(consent)
                .map_err(|e| error(ErrorCode::InvalidRequest, reason::UNKNOWN_PERMISSION, e))?,
            decided_by: DECIDED_BY.to_owned(),
        })
    }

    /// Gives the broker the plugin's effective grants (nothing unless it
    /// is enabled). Effective on its next capability call.
    ///
    /// A panic in the broker is caught: the plugin gets no grants and is
    /// disabled, and the request goes on.
    fn sync_grants(&mut self, id: &PluginId) {
        if catch_unwind(AssertUnwindSafe(|| self.apply_grants(id))).is_err() {
            warn!(plugin = %id, "the permission broker panicked; the plugin is disabled");
            if let Ok(broker_id) = deckpilot_plugin_broker::PluginId::new(id.as_str()) {
                let _ = catch_unwind(AssertUnwindSafe(|| self.broker.remove_plugin(&broker_id)));
            }
            self.stop_host(id);
            let _ = self.manager.disable(id);
            self.changed(id);
        }
    }

    fn apply_grants(&self, id: &PluginId) {
        let Ok(broker_id) = deckpilot_plugin_broker::PluginId::new(id.as_str()) else {
            return;
        };
        let enabled = self
            .manager
            .store()
            .plugin(id)
            .ok()
            .flatten()
            .is_some_and(|r| r.state == PluginState::Enabled);
        if !enabled {
            self.broker.remove_plugin(&broker_id);
            return;
        }
        let effective = self
            .manager
            .store()
            .effective_grants(id)
            .unwrap_or_default();
        let user_origins = self.user_origins(id, &effective);
        let (approved, skipped) = convert::approved_with(&effective, user_origins);
        if !skipped.is_empty() {
            info!(plugin = %id, ?skipped, "grants the broker does not enforce are not passed on");
        }
        if let Err(e) = self.broker.set_grants(&broker_id, &approved) {
            warn!(plugin = %id, error = %e, "the broker refused the grants; the plugin gets none");
            self.broker.remove_plugin(&broker_id);
            return;
        }
        // A credential slot whose host lost its network grant is unusable
        // (towards that host) until the grant is back; the rest stays.
        if let Ok(grants) = deckpilot_plugin_broker::Grants::derive(&approved) {
            for (slot, origins) in grants.suspended_credentials() {
                let origins: Vec<String> = origins.iter().map(ToString::to_string).collect();
                info!(
                    plugin = %id,
                    slot,
                    ?origins,
                    "credential slot withheld from hosts without a network grant"
                );
            }
        }
    }

    /// The consents to user-entered origins (G2) the broker may use now:
    /// only those whose setting still holds exactly that origin, within
    /// what the active manifest and the effective grants allow.
    fn user_origins(
        &self,
        id: &PluginId,
        effective: &[(GrantKey, deckpilot_plugin_store::GrantScope)],
    ) -> Vec<deckpilot_plugin_broker::ApprovedPermission> {
        let store = self.manager.store();
        let Some(manifest) = store
            .plugin(id)
            .ok()
            .flatten()
            .and_then(|r| r.active().map(|v| v.manifest.clone()))
        else {
            return Vec::new();
        };
        let rows = store.user_origins(id).unwrap_or_default();
        if rows.is_empty() {
            return Vec::new();
        }
        let saved = store.settings(id).unwrap_or_default();
        super::origins::approved(&manifest, &saved, effective, &rows)
    }

    /// Saves settings (merged, validated). A changed origin setting (G2)
    /// ends the consent to its old origin at once.
    fn set_settings(
        &mut self,
        id: &PluginId,
        mut values: BTreeMap<String, serde_json::Value>,
    ) -> Result<Response, IpcError> {
        let manifest = self.active_manifest(id)?;
        let saved = self
            .manager
            .store()
            .settings(id)
            .map_err(|e| store_error(&e))?;
        // Origin settings (G2) are saved as the broker reads them.
        super::origins::normalize(&manifest, &mut values)
            .map_err(|e| error(ErrorCode::InvalidRequest, reason::INVALID_DATA, e))?;
        let merged = data::merge_settings(&manifest, &saved, &values)
            .map_err(|e| error(ErrorCode::InvalidRequest, reason::INVALID_DATA, e))?;
        let before = super::origins::current(&manifest, &saved);
        self.manager
            .set_settings(id, &merged)
            .map_err(|e| store_error(&e))?;
        info!(plugin = %id, "plugin settings saved");
        // A changed origin setting ends the consent to its old
        // origin at once; the new one needs consent of its own.
        let after = super::origins::current(&manifest, &merged);
        let changed = super::origins::changed(&before, &after);
        for setting in &changed {
            if let Some(old) = self
                .manager
                .revoke_user_origin(id, setting, "the setting changed")
                .map_err(|e| store_error(&e))?
            {
                info!(plugin = %id, %setting, origin = %old,
                    "user origin revoked: the setting changed");
            }
        }
        if !changed.is_empty() {
            self.sync_grants(id);
            self.changed(id);
        }
        self.settings(id)
    }

    /// Consents to the origin an origin setting holds (G2): exactly that
    /// origin, a private network only when the request asks for it.
    fn grant_user_origin(
        &mut self,
        id: &PluginId,
        setting: &str,
        origin: &str,
        private_network: bool,
    ) -> Result<Response, IpcError> {
        let manifest = self.active_manifest(id)?;
        let saved = self
            .manager
            .store()
            .settings(id)
            .map_err(|e| store_error(&e))?;
        let consent = super::origins::consent(&manifest, &saved, setting, origin, private_network)
            .map_err(|e| error(ErrorCode::InvalidRequest, reason::INVALID_DATA, e))?;
        self.manager
            .grant_user_origin(id, &consent, DECIDED_BY)
            .map_err(|e| store_error(&e))?;
        info!(plugin = %id, %setting, origin = %consent.display(),
            private_network = consent.private_network, "user origin granted");
        self.sync_grants(id);
        self.changed(id);
        self.settings(id)
    }

    fn revoke_user_origin(&mut self, id: &PluginId, setting: &str) -> Result<Response, IpcError> {
        self.manager
            .revoke_user_origin(id, setting, "revoked by the user")
            .map_err(|e| store_error(&e))?;
        info!(plugin = %id, %setting, "user origin revoked");
        self.sync_grants(id);
        self.changed(id);
        self.settings(id)
    }

    /// Stops the plugin's lane: a running invocation is aborted, queued
    /// ones are refused, the host is stopped (its `dp_shutdown` runs). Every
    /// lifecycle change that stops a host (disable, update, rollback,
    /// uninstall, quarantine, crash loop) comes here, so the values the
    /// plugin published are dropped too, and a late publish from the
    /// stopped invocation is refused (a new generation).
    fn stop_host(&mut self, id: &PluginId) {
        // Its dialog closes in the app at once (`pluginStopped`); the
        // invocation that waits for it is stopped with the lane.
        self.events.dialogs.close_plugin(id.as_str());
        if let Some(lane) = self.lanes.remove(id.as_str()) {
            lane.stop();
        }
        let dropped = self.values.forget(id.as_str());
        self.announce_dropped(id.as_str(), &dropped);
    }

    /// The official revocations and the registry's verdict on the installed
    /// version; a `disable` quarantines the plugin.
    fn check_verdict(&mut self, record: &PluginRecord) -> Result<(), IpcError> {
        let Some(active) = record.active() else {
            return Ok(());
        };
        let revoked = self
            .official
            .revoked_installed(std::slice::from_ref(record));
        if let Some(why) = revoked.get(&(record.id.clone(), active.version.clone())) {
            self.stop_host(&record.id);
            if record.state != PluginState::Quarantined {
                let _ = self
                    .manager
                    .quarantine(&record.id, &format!("revoked: {why}"));
                self.sync_grants(&record.id);
                self.changed(&record.id);
            }
            return Err(error(
                ErrorCode::Failed,
                reason::REVOKED,
                format!("{} {} is not run: {why}", record.id, active.version),
            ));
        }
        let Some(registry) = self.registry.as_ref() else {
            return Ok(());
        };
        if let InstalledVerdict::Disable { reason: why, .. } = registry
            .client()
            .installed_verdict(&record.id, &active.version)
        {
            self.stop_host(&record.id);
            if record.state != PluginState::Quarantined {
                let _ = self.manager.quarantine(&record.id, &why);
                self.changed(&record.id);
            }
            return Err(error(
                ErrorCode::Failed,
                reason::REVOKED,
                format!(
                    "{} {} was revoked by the registry: {why}",
                    record.id, active.version
                ),
            ));
        }
        Ok(())
    }

    // ------------------------------------------------------------ requests

    pub fn handle(&mut self, request: Request) -> Result<Response, IpcError> {
        match request {
            Request::GetPluginRuntime => Ok(Response::PluginRuntime {
                status: Box::new(self.status()),
            }),
            Request::ListPlugins => self.list(false),
            Request::ListPluginsPaged => self.list(true),
            Request::GetPlugin { plugin } => self.details(&plugin_id(&plugin)?),
            Request::GetPluginPaged { plugin } => self.details_as(&plugin_id(&plugin)?, true),
            Request::GetPluginActions { plugin, from } => {
                self.actions_page(&plugin_id(&plugin)?, from)
            }
            Request::GetPluginIcon {
                plugin,
                offset,
                navigation,
                asset,
            } => self.icon_chunk(
                &plugin_id(&plugin)?,
                offset,
                navigation.as_ref(),
                asset.as_deref(),
            ),
            Request::ListAvailablePlugins { refresh } => self.available(refresh),
            Request::InstallPlugin { source, consent } => self.install(&source, &consent),
            Request::EnablePlugin { plugin, consent } => {
                self.enable(&plugin_id(&plugin)?, &consent)
            }
            Request::DisablePlugin { plugin } => {
                let id = plugin_id(&plugin)?;
                self.guard_removal(&id, deckpilot_plugin_store::deps::Removal::Disable)?;
                self.disable(&id)
            }
            Request::GrantPluginPermissions {
                plugin,
                permissions,
            } => self.grant(&plugin_id(&plugin)?, &permissions),
            Request::RevokePluginPermission { plugin, permission } => {
                self.revoke(&plugin_id(&plugin)?, &permission)
            }
            Request::UpdatePlugin {
                plugin,
                version,
                consent,
            } => self.update(&plugin_id(&plugin)?, version.as_deref(), &consent),
            Request::ApprovePluginUpdate { plugin, consent } => {
                self.approve(&plugin_id(&plugin)?, &consent)
            }
            Request::DeclinePluginUpdate { plugin } => {
                let id = plugin_id(&plugin)?;
                self.manager
                    .decline_update(&id)
                    .map_err(|e| store_error(&e))?;
                self.changed(&id);
                self.details(&id)
            }
            Request::RollbackPlugin { plugin } => self.rollback(&plugin_id(&plugin)?),
            Request::UninstallPlugin { plugin, keep } => {
                let id = plugin_id(&plugin)?;
                self.guard_removal(&id, deckpilot_plugin_store::deps::Removal::Uninstall)?;
                self.uninstall(
                    &id,
                    Retention {
                        settings: keep.settings,
                        credentials: keep.credentials,
                    },
                )
            }
            Request::RestoreBundledPlugin { plugin } => self.restore_bundled(&plugin_id(&plugin)?),
            request @ (Request::PlanPluginInstall { .. }
            | Request::InstallPluginPlan { .. }
            | Request::PlanPluginRemoval { .. }
            | Request::RemovePlugin { .. }) => self.handle_dependencies(request),
            other => self.handle_data(other),
        }
    }

    /// Requests about a plugin's data: known plugins, settings,
    /// credentials.
    fn handle_data(&mut self, request: Request) -> Result<Response, IpcError> {
        match request {
            Request::ListKnownPlugins => {
                let store = self.manager.store();
                let installed = store.plugins().map_err(|e| store_error(&e))?;
                Ok(Response::KnownPlugins {
                    plugins: store
                        .known()
                        .map_err(|e| store_error(&e))?
                        .iter()
                        .map(|k| {
                            let present = installed.iter().any(|r| r.id == k.id);
                            let restorable =
                                !present && bundled::restorable(&self.manager, &self.bundle, &k.id);
                            data::known(k, present, restorable)
                        })
                        .collect(),
                })
            }
            Request::GetPluginSettings { plugin } => self.settings(&plugin_id(&plugin)?),
            Request::SetPluginSettings { plugin, values } => {
                self.set_settings(&plugin_id(&plugin)?, values)
            }
            Request::GrantPluginUserOrigin {
                plugin,
                setting,
                origin,
                private_network,
            } => self.grant_user_origin(&plugin_id(&plugin)?, &setting, &origin, private_network),
            Request::RevokePluginUserOrigin { plugin, setting } => {
                self.revoke_user_origin(&plugin_id(&plugin)?, &setting)
            }
            Request::ListPluginCredentials { plugin } => self.credentials(&plugin_id(&plugin)?),
            Request::SetPluginCredential {
                plugin,
                key,
                secret,
            } => {
                let id = plugin_id(&plugin)?;
                self.manager
                    .set_credential(&id, &key, secret.expose())
                    .map_err(|e| store_error(&e))?;
                // The slot only; never the secret.
                info!(plugin = %id, credential = %key, "plugin credential stored");
                self.credentials(&id)
            }
            Request::DeletePluginCredential { plugin, key } => {
                let id = plugin_id(&plugin)?;
                self.manager
                    .delete_credential(&id, &key)
                    .map_err(|e| store_error(&e))?;
                info!(plugin = %id, credential = %key, "plugin credential deleted");
                self.credentials(&id)
            }
            Request::ListModuleMigrations => Ok(Response::ModuleMigrations {
                migrations: super::migration::infos(
                    &self.manager,
                    &self.bundle,
                    self.snapshot.as_ref(),
                ),
            }),
            Request::DecideModuleMigration {
                module,
                approve,
                decline,
            } => {
                let keys = |items: &[String]| {
                    convert::keys(items).map_err(|e| {
                        error(ErrorCode::InvalidRequest, reason::UNKNOWN_PERMISSION, e)
                    })
                };
                let decision = super::migration::Decision::Decide {
                    module,
                    approve: keys(&approve)?,
                    decline: keys(&decline)?,
                };
                self.decide_migration(&decision)
            }
            Request::RetryModuleMigration { module } => {
                self.decide_migration(&super::migration::Decision::Retry(module))
            }
            other => Err(IpcError::new(
                ErrorCode::InvalidRequest,
                format!("`{}` is not a plugin request", other.name()),
            )),
        }
    }

    fn available(&mut self, refresh: bool) -> Result<Response, IpcError> {
        if refresh {
            self.refresh()?;
            self.apply_verdicts();
        }
        let registry = self.registry()?;
        let host = host_environment();
        let installed: BTreeMap<String, String> = self
            .manager
            .store()
            .plugins()
            .map_err(|e| store_error(&e))?
            .iter()
            .filter_map(|r| Some((r.id.to_string(), r.active()?.version.to_string())))
            .collect();
        let graph = self.graph();
        let mut plugins = Vec::new();
        if let Some(index) = registry.client().index() {
            for entry in &index.plugins {
                let detail = match registry.detail(&entry.id) {
                    Ok(detail) => detail,
                    Err(problem) => {
                        warn!(plugin = %entry.id, %problem, "registry detail refused");
                        continue;
                    }
                };
                let mut versions: Vec<ipc::AvailableVersion> = detail
                    .versions
                    .iter()
                    .map(|v| {
                        let revoked = index
                            .revocations
                            .iter()
                            .find(|r| r.matches(&entry.id, &v.version))
                            .map(|r| r.reason.clone());
                        let mut available = available_version(v, revoked, host.as_ref());
                        available.dependencies = self.available_dependencies(
                            v.dependencies.as_deref().unwrap_or_default(),
                            &graph,
                        );
                        available
                    })
                    .collect();
                versions.reverse();
                let publisher = index
                    .publishers
                    .iter()
                    .find(|p| p.namespace == entry.publisher)
                    .map(|p| ipc::PublisherInfo {
                        namespace: p.namespace.clone(),
                        display_name: p.display_name.clone(),
                        login: p.login.clone(),
                        verified: p.verified,
                    });
                plugins.push(ipc::AvailablePlugin {
                    id: entry.id.to_string(),
                    name: entry.name.clone(),
                    summary: entry.summary.clone(),
                    publisher: entry.publisher.clone(),
                    latest: entry.latest.stable.as_ref().map(ToString::to_string),
                    versions,
                    installed: installed.get(entry.id.as_str()).cloned(),
                    description: Some(detail.description.clone()),
                    categories: entry.categories.clone().unwrap_or_default(),
                    publisher_info: publisher,
                    license: Some(detail.license.clone()),
                    homepage: detail.homepage.clone(),
                    source_repository: detail.source_repository.clone(),
                    deprecated: entry.deprecated.as_ref().map(|d| d.reason.clone()),
                });
            }
        }
        Ok(Response::AvailablePlugins {
            registry: registry.status(),
            plugins,
        })
    }

    fn install(
        &mut self,
        source: &PluginInstallSource,
        consent: &[String],
    ) -> Result<Response, IpcError> {
        let label = match source {
            PluginInstallSource::Registry { plugin, .. } => plugin.clone(),
            PluginInstallSource::File { path } => path.clone(),
        };
        let result = self.install_steps(source, consent);
        match &result {
            Ok(Response::Plugin { plugin }) => {
                self.progress(
                    &plugin.summary.id,
                    PluginOperation::Install,
                    PluginStage::Finished,
                );
            }
            Err(e) => self.failed(&label, PluginOperation::Install, e),
            Ok(_) => {}
        }
        result
    }

    fn install_steps(
        &mut self,
        source: &PluginInstallSource,
        consent: &[String],
    ) -> Result<Response, IpcError> {
        let consent = Self::consent(consent)?;
        let (id, bytes, expected) = match source {
            PluginInstallSource::Registry { plugin, version } => {
                let id = plugin_id(plugin)?;
                self.progress(id.as_str(), PluginOperation::Install, PluginStage::Fetching);
                self.refresh()?;
                let version = self.pick_version(&id, version.as_deref())?;
                let (expected, bytes) = self.fetch(&id, &version)?;
                (id, bytes, expected)
            }
            PluginInstallSource::File { path } => {
                self.progress(path, PluginOperation::Install, PluginStage::Fetching);
                let bytes = read_package_file(Path::new(path))?;
                // Which plugin and version the file claims to be; the
                // registry then decides whether these exact bytes are it.
                let files =
                    read_archive(&bytes, &Limits::default()).map_err(|e| package_error(&e))?;
                let manifest = files
                    .get(MANIFEST)
                    .ok_or_else(|| package_error(&PackageError::Missing(MANIFEST)))
                    .and_then(|bytes| {
                        Manifest::from_slice(bytes).map_err(|e| package_error(&e.into()))
                    })?;
                if manifest.manifest_version != MANIFEST_VERSION {
                    return Err(invalid("unsupported manifest version"));
                }
                // First-party plugins come only from the application bundle
                // or a registry, never from a file.
                if deckpilot_plugin_store::is_first_party(&manifest.id) {
                    return Err(store_error(&StoreError::SourceNotAllowed {
                        id: manifest.id,
                        origin: InstallSource::Sideload.as_str(),
                    }));
                }
                // The artifact contract first: a JavaScript package is
                // refused as such, whatever the registry says.
                if manifest.runtime_contract() == RuntimeContract::LegacyJs {
                    return Err(package_error(&PackageError::UnsupportedRuntime {
                        found: if files.contains_key(LEGACY_JS_ARTIFACT) {
                            "a JavaScript bundle (plugin.js) without a `runtime` declaration".into()
                        } else {
                            "no `runtime` declaration".into()
                        },
                    }));
                }
                self.refresh()?;
                let mut expected = self.pins(&manifest.id, &manifest.version)?.expected;
                // A file is never first-party (refused above); the
                // first-party rules apply only to a registry download.
                expected.first_party = false;
                (manifest.id, bytes, expected)
            }
        };
        self.progress(
            id.as_str(),
            PluginOperation::Install,
            PluginStage::Verifying,
        );
        let verified =
            verify_package(&bytes, &expected, &Limits::default()).map_err(|e| package_error(&e))?;
        self.progress(
            id.as_str(),
            PluginOperation::Install,
            PluginStage::Installing,
        );
        let source = self.install_source();
        self.manager
            .install(&verified, &source, &consent)
            .map_err(|e| store_error(&e))?;
        info!(plugin = %id, version = %expected.version, "plugin installed");
        self.changed(&id);
        self.details(&id)
    }

    fn enable(&mut self, id: &PluginId, consent: &[String]) -> Result<Response, IpcError> {
        platform_gate()?;
        let record = self.record(id)?;
        self.check_verdict(&record)?;
        self.progress(id.as_str(), PluginOperation::Enable, PluginStage::Started);
        let consent = Self::consent(consent)?;
        if !consent.keys.is_empty() {
            self.manager
                .grant(id, &consent)
                .map_err(|e| store_error(&e))?;
        }
        // The user enabling a first-party plugin they disabled is the one
        // way its tombstone goes (seeding never clears it).
        if let Ok(Some(tombstone)) = self.manager.store().tombstone(id)
            && tombstone.decision == deckpilot_plugin_store::TombstoneDecision::Disabled
        {
            self.manager
                .clear_tombstone(id)
                .map_err(|e| store_error(&e))?;
        }
        let result = self.manager.enable(id).map_err(|e| store_error(&e));
        if let Err(e) = &result {
            self.failed(id.as_str(), PluginOperation::Enable, e);
            self.changed(id);
            result?;
        }
        // A lane that tripped the crash-loop breaker is already gone, so the
        // next invocation starts with a fresh supervisor (no crash history).
        self.sync_grants(id);
        self.progress(id.as_str(), PluginOperation::Enable, PluginStage::Finished);
        self.changed(id);
        self.details(id)
    }

    fn disable(&mut self, id: &PluginId) -> Result<Response, IpcError> {
        self.stop_host(id);
        // The user's choice: a first-party plugin keeps it (a tombstone),
        // so seeding never enables it again.
        self.manager
            .disable_by_user(id)
            .map_err(|e| store_error(&e))?;
        self.sync_grants(id);
        self.changed(id);
        self.details(id)
    }

    fn grant(&mut self, id: &PluginId, permissions: &[String]) -> Result<Response, IpcError> {
        let consent = Self::consent(permissions)?;
        self.manager
            .grant(id, &consent)
            .map_err(|e| store_error(&e))?;
        self.sync_grants(id);
        self.changed(id);
        self.details(id)
    }

    fn revoke(&mut self, id: &PluginId, permission: &str) -> Result<Response, IpcError> {
        let key: GrantKey = permission
            .parse()
            .map_err(|e: String| error(ErrorCode::InvalidRequest, reason::UNKNOWN_PERMISSION, e))?;
        self.manager
            .revoke(id, &key, DECIDED_BY)
            .map_err(|e| store_error(&e))?;
        // Revoking an origin setting's permission (G2) also ends the
        // consent to its server: granting it again asks again.
        if let Some(setting) = deckpilot_plugin_store::user_origin_setting(&key) {
            self.manager
                .revoke_user_origin(id, setting, "its permission was revoked")
                .map_err(|e| store_error(&e))?;
        }
        self.sync_grants(id);
        self.changed(id);
        self.details(id)
    }

    fn update(
        &mut self,
        id: &PluginId,
        requested: Option<&str>,
        consent: &[String],
    ) -> Result<Response, IpcError> {
        let consent = Self::consent(consent)?;
        let record = self.record(id)?;
        self.progress(id.as_str(), PluginOperation::Update, PluginStage::Fetching);
        let result = (|| {
            self.refresh()?;
            let version = self.pick_version(id, requested)?;
            if record.active().is_some_and(|a| a.version >= version) {
                return Err(error(
                    ErrorCode::Failed,
                    reason::INVALID_STATE,
                    format!("{id} {version} is not newer than the installed version"),
                ));
            }
            // Dependencies (plugin API 1.4): never a version another
            // installed plugin refuses, never one whose dependencies are
            // not there.
            self.check_update(id, &version)?;
            let (expected, bytes) = self.fetch(id, &version)?;
            self.progress(id.as_str(), PluginOperation::Update, PluginStage::Verifying);
            let verified = verify_package(&bytes, &expected, &Limits::default())
                .map_err(|e| package_error(&e))?;
            // Bundled → Marketplace-managed: the update replaces the bundled
            // copy only if it keeps what the user's configuration refers to;
            // the bundled copy stays active until the store committed it.
            if record.source == InstallSource::Bundled
                && let Some(active) = record.active()
            {
                official::check_transition(&active.manifest, verified.manifest())
                    .map_err(|m| error(ErrorCode::Failed, reason::INVALID_PACKAGE, m))?;
            }
            self.trial_load(id, &verified)?;
            self.progress(
                id.as_str(),
                PluginOperation::Update,
                PluginStage::Installing,
            );
            let source = self.install_source();
            self.manager
                .update(&verified, &source)
                .map_err(|e| store_error(&e))
        })();
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(e) => {
                self.failed(id.as_str(), PluginOperation::Update, &e);
                self.changed(id);
                return Err(e);
            }
        };
        let outcome = match outcome {
            UpdateOutcome::Applied { version, diff } => {
                self.after_version_change(id);
                PluginUpdateOutcome::Applied {
                    version: version.to_string(),
                    changes: convert::changes(&diff, &self.shown_manifest(id)?),
                }
            }
            UpdateOutcome::AwaitingConsent { version, diff } => {
                let escalations: BTreeSet<GrantKey> = diff.escalations().into_iter().collect();
                if !escalations.is_empty() && escalations.is_subset(&consent.keys) {
                    let changes = convert::changes(&diff, &self.shown_staged_manifest(id)?);
                    self.manager
                        .approve_update(id, &consent)
                        .map_err(|e| store_error(&e))?;
                    self.after_version_change(id);
                    PluginUpdateOutcome::Applied {
                        version: version.to_string(),
                        changes,
                    }
                } else {
                    PluginUpdateOutcome::AwaitingConsent {
                        version: version.to_string(),
                        changes: convert::changes(&diff, &self.shown_staged_manifest(id)?),
                    }
                }
            }
        };
        self.progress(id.as_str(), PluginOperation::Update, PluginStage::Finished);
        self.changed(id);
        Ok(Response::PluginUpdate {
            plugin: id.to_string(),
            outcome,
        })
    }

    /// Loads the module of an update in a host of its own, as the next
    /// invocation would, before anything is installed: an update whose
    /// module the host refuses (actions other than the manifest's, a
    /// failing `dp_init`, a broken descriptor) is refused, so the running
    /// version stays active instead of being replaced by one that cannot
    /// run. Problems of this computer rather than of the module (no host
    /// sandbox, a host that cannot start) do not block the update: they
    /// would refuse the running version alike.
    fn trial_load(
        &self,
        id: &PluginId,
        verified: &deckpilot_plugin_package::VerifiedPackage,
    ) -> Result<(), IpcError> {
        if platform_gate().is_err() {
            return Ok(());
        }
        let Some(module) = verified.files().get(deckpilot_plugin_package::ARTIFACT) else {
            return Ok(());
        };
        let Ok(broker_id) = deckpilot_plugin_broker::PluginId::new(id.as_str()) else {
            return Ok(());
        };
        let actions = verified
            .manifest()
            .contributes
            .actions()
            .iter()
            .map(|a| a.id.clone());
        let config = host_config(&self.host, Arc::from(module.as_slice()), actions);
        let mut supervisor = Supervisor::new(
            config,
            self.broker.clone(),
            broker_id,
            RestartPolicy::default(),
            Arc::new(SystemClock::default()),
        );
        let started = supervisor.start();
        supervisor.shutdown();
        match started {
            Ok(()) => Ok(()),
            Err(SupervisorError::Host(
                e @ (HostError::ActionMismatch { .. }
                | HostError::LoadFailed { .. }
                | HostError::Protocol { .. }
                | HostError::ModuleTooLarge),
            )) => {
                let version = &verified.manifest().version;
                warn!(plugin = %id, %version, error = %e, "update refused: the host refused its module");
                Err(error(
                    ErrorCode::Failed,
                    reason::BROKEN,
                    format!(
                        "{id} {version} was not installed: the plugin host refused its module \
                         ({e}); the installed version stays"
                    ),
                ))
            }
            Err(e) => {
                warn!(plugin = %id, error = %e, "the update's module could not be tried");
                Ok(())
            }
        }
    }

    fn approve(&mut self, id: &PluginId, consent: &[String]) -> Result<Response, IpcError> {
        let consent = Self::consent(consent)?;
        let (_, diff) = self
            .manager
            .pending_update(id)
            .map_err(|e| store_error(&e))?
            .ok_or_else(|| store_error(&StoreError::NoStagedUpdate(id.clone())))?;
        let changes = convert::changes(&diff, &self.shown_staged_manifest(id)?);
        let version = self
            .manager
            .approve_update(id, &consent)
            .map_err(|e| store_error(&e))?;
        self.after_version_change(id);
        self.changed(id);
        Ok(Response::PluginUpdate {
            plugin: id.to_string(),
            outcome: PluginUpdateOutcome::Applied {
                version: version.to_string(),
                changes,
            },
        })
    }

    fn rollback(&mut self, id: &PluginId) -> Result<Response, IpcError> {
        self.check_rollback(id)?;
        self.stop_host(id);
        // The version rolled back to must pass the official revocations
        // too (a revoked release, a revoked signer, no record of it).
        let records = self.manager.store().plugins().unwrap_or_default();
        let revoked = self.official.revoked_installed(&records);
        let verdicts = official::Verdicts {
            official: &revoked,
            registry: match self.registry.as_ref() {
                Some(registry) => registry.client(),
                None => &NoRevocations,
            },
        };
        let result = self.manager.rollback(id, &verdicts);
        result.map_err(|e| store_error(&e))?;
        self.after_version_change(id);
        self.changed(id);
        self.details(id)
    }

    fn settings(&self, id: &PluginId) -> Result<Response, IpcError> {
        let manifest = self.shown_manifest(id)?;
        let store = self.manager.store();
        let saved = store.settings(id).map_err(|e| store_error(&e))?;
        let grants = store.grants(id).map_err(|e| store_error(&e))?;
        let rows = store.user_origins(id).map_err(|e| store_error(&e))?;
        let origins = super::origins::infos(&manifest, &saved, &grants, &rows);
        let mut settings = data::settings(&manifest, saved);
        settings.origins = origins;
        Ok(Response::PluginSettings {
            settings: Box::new(settings),
        })
    }

    fn credentials(&self, id: &PluginId) -> Result<Response, IpcError> {
        let manifest = self.shown_manifest(id)?;
        let store = self.manager.store();
        let stored = store.credentials(id).map_err(|e| store_error(&e))?;
        let grants = store.grants(id).map_err(|e| store_error(&e))?;
        Ok(Response::PluginCredentials {
            plugin: id.to_string(),
            credentials: data::credential_slots(&manifest, &stored, &grants),
        })
    }

    /// Uninstall (design §5.6): the host stops, the broker forgets the
    /// plugin, then the store deletes files, records, grants and storage,
    /// and settings and credentials unless `keep` names them;
    /// `plugin_known` and the audit stay.
    fn uninstall(&mut self, id: &PluginId, keep: Retention) -> Result<Response, IpcError> {
        self.progress(
            id.as_str(),
            PluginOperation::Uninstall,
            PluginStage::Started,
        );
        self.stop_host(id);
        if let Ok(broker_id) = deckpilot_plugin_broker::PluginId::new(id.as_str()) {
            self.broker.remove_plugin(&broker_id);
        }
        // The user's choice: a first-party plugin keeps it (a tombstone),
        // so seeding never installs it again.
        self.manager
            .uninstall_by_user(id, keep)
            .map_err(|e| store_error(&e))?;
        self.health.forget(id.as_str());
        self.locales.forget(id.as_str());
        info!(plugin = %id, ?keep, "plugin uninstalled");
        self.progress(
            id.as_str(),
            PluginOperation::Uninstall,
            PluginStage::Finished,
        );
        let _ = self.events.plugins.send(PluginEvent::Changed {
            plugin: id.to_string(),
            state: None,
        });
        Ok(Response::Ok)
    }

    /// `restoreBundledPlugin`: a bundled plugin the user uninstalled is
    /// installed again from the application bundle (its tombstone
    /// cleared), or a broken bundled copy is replaced by the bundle's; both
    /// verified against the pins compiled into the agent before anything
    /// changes ([`bundled::restore`]). Progress arrives as `install`
    /// events; the migration of a module the plugin replaces routes to it
    /// again (the worker publishes the routing after every request).
    fn restore_bundled(&mut self, id: &PluginId) -> Result<Response, IpcError> {
        use bundled::RestoreRefusal;
        platform_gate()?;
        self.progress(
            id.as_str(),
            PluginOperation::Install,
            PluginStage::Verifying,
        );
        let broken = matches!(
            self.manager.store().plugin(id),
            Ok(Some(record)) if record.state == PluginState::Broken
        );
        if broken {
            // The broken copy's host, values and dialogs go before its
            // files do.
            self.stop_host(id);
        }
        let result = bundled::restore(&mut self.manager, &self.bundle, id).map_err(|refusal| {
            match refusal {
                RestoreRefusal::NotBundled(why) => {
                    error(ErrorCode::InvalidRequest, reason::NOT_BUNDLED, why)
                }
                RestoreRefusal::NotPinned(why) => {
                    error(ErrorCode::NotFound, reason::NOT_PINNED, why)
                }
                RestoreRefusal::Installed(why) => {
                    error(ErrorCode::Failed, reason::ALREADY_INSTALLED, why)
                }
                RestoreRefusal::InvalidState(why) => {
                    error(ErrorCode::Failed, reason::INVALID_STATE, why)
                }
                RestoreRefusal::Verification(why) => error(
                    ErrorCode::Failed,
                    reason::INVALID_PACKAGE,
                    format!(
                        "the bundled copy of {id} failed verification; nothing was changed: {why}"
                    ),
                ),
                RestoreRefusal::Store(e) => store_error(&e),
            }
        });
        let restored = match result {
            Ok(restored) => restored,
            Err(e) => {
                self.failed(id.as_str(), PluginOperation::Install, &e);
                // A failed reinstall may have changed the plugin's state.
                if broken {
                    self.changed(id);
                }
                return Err(e);
            }
        };
        info!(plugin = %id, ?restored, "bundled plugin restored by the user");
        self.health.forget(id.as_str());
        self.after_version_change(id);
        self.progress(id.as_str(), PluginOperation::Install, PluginStage::Finished);
        self.changed(id);
        self.details(id)
    }

    /// The running module changed: its host is restarted on demand.
    fn after_version_change(&mut self, id: &PluginId) {
        self.stop_host(id);
        // Another version: its own verified translations, read again.
        self.locales.forget(id.as_str());
        self.forget_undeclared_origins(id);
        self.sync_grants(id);
    }

    /// Deletes the consents to servers the user entered (G2) whose origin
    /// setting the active version no longer declares: a later version that
    /// declares it again asks again, never revives an old consent.
    fn forget_undeclared_origins(&mut self, id: &PluginId) {
        let Ok(manifest) = self.active_manifest(id) else {
            return;
        };
        let rows = self.manager.store().user_origins(id).unwrap_or_default();
        for row in rows {
            let setting = &row.consent.setting;
            if deckpilot_plugin_store::declared_slot(&manifest, setting).is_none()
                && let Err(e) = self.manager.revoke_user_origin(
                    id,
                    setting,
                    "the version no longer declares the setting",
                )
            {
                warn!(plugin = %id, %setting, error = %e, "a user origin could not be forgotten");
            }
        }
    }

    fn active_manifest(&self, id: &PluginId) -> Result<Manifest, IpcError> {
        self.record(id)?
            .active()
            .map(|v| v.manifest.clone())
            .ok_or_else(|| store_error(&StoreError::NotInstalled(id.clone())))
    }

    /// The active manifest in the app's language (for what it shows).
    fn shown_manifest(&self, id: &PluginId) -> Result<Manifest, IpcError> {
        self.shown(&self.record(id)?)
            .active()
            .map(|v| v.manifest.clone())
            .ok_or_else(|| store_error(&StoreError::NotInstalled(id.clone())))
    }

    /// The staged manifest in the app's language.
    fn shown_staged_manifest(&self, id: &PluginId) -> Result<Manifest, IpcError> {
        self.shown(&self.record(id)?)
            .staged()
            .map(|v| v.manifest.clone())
            .ok_or_else(|| store_error(&StoreError::NoStagedUpdate(id.clone())))
    }

    // ---------------------------------------------------------- invocation

    /// The plugin's lane, started with a new supervisor for the active
    /// version's module (re-verified against its integrity record) when
    /// there is none.
    fn lane(&mut self, record: &PluginRecord) -> Result<&Lane, IpcError> {
        let key = record.id.to_string();
        if !self.lanes.contains_key(&key) {
            let active = record
                .active()
                .ok_or_else(|| store_error(&StoreError::NotInstalled(record.id.clone())))?;
            // The files must still be what was installed (a mismatch marks
            // the plugin broken), and the module must still be ABI 1.
            self.manager
                .verify(&record.id)
                .map_err(|e| store_error(&e))?;
            let dir = self.manager.layout().resolve(&active.install_dir);
            // A bundled copy is also checked against the pins compiled into
            // the agent, and so are the very module bytes the host gets.
            let pin = bundled::check_installed(&mut self.manager, &self.bundle, record, &dir)
                .map_err(|why| {
                    self.changed(&record.id);
                    error(ErrorCode::Failed, reason::BROKEN, why)
                })?
                .cloned();
            let path = dir.join(deckpilot_plugin_package::ARTIFACT);
            let module = std::fs::read(&path).map_err(|e| {
                error(
                    ErrorCode::Failed,
                    reason::BROKEN,
                    format!("the plugin module cannot be read: {e}"),
                )
            })?;
            // `verify` read the files once; the host gets these bytes, read
            // again, so they are checked themselves: a module swapped on disk
            // in between is never compiled.
            if let Err(why) = module_matches_record(&active.integrity, &module) {
                let _ = self.manager.mark_broken(&record.id, &why);
                self.changed(&record.id);
                return Err(error(ErrorCode::Failed, reason::BROKEN, why));
            }
            bundled::check_module(&mut self.manager, record, pin.as_ref(), &module).map_err(
                |why| {
                    self.changed(&record.id);
                    error(ErrorCode::Failed, reason::BROKEN, why)
                },
            )?;
            deckpilot_plugin_package::wasm::validate_module(&module).map_err(|e| {
                error(
                    ErrorCode::Failed,
                    reason::UNSUPPORTED_RUNTIME,
                    e.to_string(),
                )
            })?;
            let actions = active
                .manifest
                .contributes
                .actions()
                .iter()
                .map(|a| a.id.clone());
            let config = host_config(&self.host, Arc::from(module), actions);
            let broker_id = deckpilot_plugin_broker::PluginId::new(record.id.as_str())
                .map_err(|_| invalid("the plugin id is not valid for the broker"))?;
            let supervisor = Supervisor::new(
                config,
                self.broker.clone(),
                broker_id,
                RestartPolicy::default(),
                Arc::new(SystemClock::default()),
            );
            self.next_lane += 1;
            let lane = Lane::start(
                key.clone(),
                self.next_lane,
                supervisor,
                self.lane_ended.clone(),
            );
            self.lanes.insert(key.clone(), lane);
        }
        Ok(self.lanes.get(&key).expect("the lane was just inserted"))
    }

    /// Checks that `action` of `id` may run now and prepares it. With
    /// `engine`, every declared permission must be granted too (the
    /// catalog's rule for keys and workflows).
    fn prepare(
        &mut self,
        id: &PluginId,
        action: &str,
        engine: bool,
    ) -> Result<(PluginRecord, Prepared), IpcError> {
        platform_gate()?;
        let record = self.record(id)?;
        if record.state != PluginState::Enabled {
            return Err(error(
                ErrorCode::Failed,
                reason::NOT_ENABLED,
                format!(
                    "{id} is not enabled ({})",
                    convert::unavailable(&record).unwrap_or_default()
                ),
            ));
        }
        if engine {
            let grants = self
                .manager
                .store()
                .grants(id)
                .map_err(|e| store_error(&e))?;
            if let Some(why) = convert::unavailable_with(&record, &grants) {
                return Err(error(ErrorCode::Failed, reason::NOT_ENABLED, why));
            }
            if let Some(why) = record
                .active()
                .and_then(|a| convert::unmet_permissions(&a.manifest, &grants).remove(action))
            {
                return Err(error(ErrorCode::Failed, reason::NOT_ENABLED, why));
            }
        }
        self.check_verdict(&record)?;
        let active = record
            .active()
            .ok_or_else(|| store_error(&StoreError::NotInstalled(id.clone())))?;
        let declaration = active
            .manifest
            .contributes
            .actions()
            .iter()
            .find(|a| a.id == action)
            .cloned()
            .ok_or_else(|| {
                IpcError::new(
                    ErrorCode::NotFound,
                    format!("{id} has no action `{action}`"),
                )
            })?;
        // Dependencies (plugin API 1.4): a required one missing stops every
        // action, an optional one only the actions that name it.
        let (missing, unmet) = self.unmet(&active.manifest, &self.graph());
        if let Some(why) = missing.or_else(|| unmet.get(action).cloned()) {
            return Err(error(ErrorCode::Failed, reason::NOT_ENABLED, why));
        }
        let node = declaration.node.clone().unwrap_or_default();
        let timeout = Duration::from_millis(node.timeout_ms()).min(MAX_INVOCATION);
        let outputs = super::lane::declared_outputs(&declaration);
        let resource_kinds = super::executor::resources::declared(&declaration);
        let version = active.manifest.version.to_string();
        let dir = self.manager.layout().resolve(&active.install_dir);
        // Notifications and dialogs are titled with the name the app shows.
        let name = self.locales.manifest(id.as_str(), active, &dir).name;
        let translator = self.locales.translator(id.as_str(), active, &dir);
        let definition = super::engine::action_type(id.as_str(), &declaration).map_err(|e| {
            error(
                ErrorCode::Failed,
                reason::BROKEN,
                format!("the action `{action}` of {id} cannot be offered: {e}"),
            )
        })?;
        let access = DataAccess {
            data: self.data.clone(),
            manifest: Arc::new(active.manifest.clone()),
        };
        let variables = OwnVariables {
            store: self.values.clone(),
            namespace: active.manifest.id.variable_namespace(),
            generation: self.values.generation(id.as_str()),
            declared: active.manifest.contributes.variables().to_vec(),
        };
        self.sync_grants(id);
        let executor = Executor::new(
            self.broker.clone(),
            name,
            self.events.agent.clone(),
            self.transport.clone(),
            variables,
            self.opener.clone(),
        )
        .with_notifier(self.events.notifier.clone())
        .with_dialogs(self.events.dialogs.clone())
        .with_translator(translator)
        .with_data(access)
        .with_projects(self.events.projects.clone());
        Ok((
            record,
            Prepared {
                timeout,
                outputs,
                executor,
                definition,
                resource_kinds,
                version,
            },
        ))
    }

    /// `invokePluginAction`: answered on the plugin's lane when the
    /// action finished.
    pub fn invoke(
        &mut self,
        plugin: &str,
        action: &str,
        params: &BTreeMap<String, serde_json::Value>,
        reply: oneshot::Sender<Result<Response, IpcError>>,
    ) {
        let started = (|| {
            let id = plugin_id(plugin)?;
            let params = convert::params(params).map_err(invalid)?;
            let (record, prepared) = self.prepare(&id, action, false)?;
            Ok::<_, IpcError>((id, record, params, prepared))
        })();
        let (id, record, params, prepared) = match started {
            Ok(started) => started,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        let Prepared {
            timeout,
            outputs,
            executor,
            definition,
            ..
        } = prepared;
        // Checked against the manifest before the host sees anything. A
        // resource parameter cannot be given here (no key or workflow
        // resolves it).
        let params = match data::missing_resource(&definition, |_| false)
            .map_or_else(|| data::normalize_params(&definition, &params), Err)
        {
            Ok(params) => params,
            Err(problem) => {
                let _ = reply.send(Ok(Response::PluginActionResult {
                    outcome: ipc::PluginActionOutcome::Failed {
                        code: INVALID_PARAMETERS.to_owned(),
                        message: Some(problem),
                        retryable: false,
                    },
                }));
                return;
            }
        };
        let plugin = id.to_string();
        let name = action.to_owned();
        let done: Done = Box::new(move |result| {
            let answer = match result {
                Ok(outcome) => Ok(Response::PluginActionResult {
                    outcome: convert::outcome(outcome, &outputs),
                }),
                Err(e) => {
                    warn!(plugin, action = name, error = ?e, "plugin invocation failed");
                    Err(lane_error(&e))
                }
            };
            let _ = reply.send(answer);
        });
        let done = self.health.watch(id.to_string(), done);
        match self.lane(&record) {
            Ok(lane) => lane.submit(Job {
                action: action.to_owned(),
                params,
                timeout,
                // Tried from the plugin's own page in the app (G8).
                executor: executor.with_run(RunContext::direct(RunTrigger::Test)),
                abort: AbortSignal::new(),
                reply: Reply::Action(done),
            }),
            Err(e) => done(Err(LaneError::Refused(e))),
        }
    }

    /// A plugin action from the workflow engine (a key, `runAction`, a
    /// workflow node), of the run `run` describes. The answer goes to
    /// `reply`; a caller that gave up has dropped its end, and the answer
    /// is discarded.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the worker hands its message's parameters over; they are validated into new ones"
    )]
    #[expect(
        clippy::too_many_arguments,
        reason = "one worker message, handed over field by field"
    )]
    pub fn invoke_for_engine(
        &mut self,
        plugin: &str,
        action: &str,
        params: Params,
        resources: InvocationResources,
        run: RunContext,
        abort: AbortSignal,
        reply: std::sync::mpsc::SyncSender<EngineReply>,
    ) {
        let fail = |e: &IpcError| EngineFailure {
            code: engine_code(e.reason.as_deref()).to_owned(),
            message: e.message.clone(),
        };
        let prepared = plugin_id(plugin).and_then(|id| self.prepare(&id, action, true));
        let (record, prepared) = match prepared {
            Ok(prepared) => prepared,
            Err(e) => {
                let _ = reply.try_send(Err(fail(&e)));
                return;
            }
        };
        let Prepared {
            timeout,
            outputs,
            executor,
            definition,
            resource_kinds,
            version,
        } = prepared;
        let mut params =
            match data::missing_resource(&definition, |key| resources.params.contains_key(key))
                .map_or_else(|| data::normalize_params(&definition, &params), Err)
            {
                Ok(params) => params,
                Err(message) => {
                    let _ = reply.try_send(Err(EngineFailure {
                        code: INVALID_PARAMETERS.to_owned(),
                        message,
                    }));
                    return;
                }
            };
        // Resource parameters reach the plugin as handles of this
        // invocation only (plugin API 1.9, X1); with the Projects feature
        // off no project role resolves.
        let resources = if self.projects_enabled() {
            resources
        } else {
            resources.without_projects()
        };
        let (executor, handles) =
            executor.with_resources(record.id.as_str(), &version, resource_kinds, resources);
        params.extend(
            handles
                .into_iter()
                .map(|(key, handle)| (key, ParamValue::String(handle))),
        );
        let id = record.id.to_string();
        let name = action.to_owned();
        let done: Done = Box::new(move |result| {
            let answer = match result {
                Ok(outcome) => Ok(filter_outputs(outcome, &outputs)),
                Err(e) => {
                    warn!(plugin = id, action = name, error = ?e, "plugin action failed");
                    Err(fail(&lane_error(&e)))
                }
            };
            let _ = reply.try_send(answer);
        });
        let done = self.health.watch(record.id.to_string(), done);
        match self.lane(&record) {
            Ok(lane) => lane.submit(Job {
                action: action.to_owned(),
                params,
                timeout,
                executor: executor.with_run(run),
                abort,
                reply: Reply::Action(done),
            }),
            Err(e) => done(Err(LaneError::Refused(e))),
        }
    }

    /// Whether the Projects feature is on: the Projects plugin
    /// (`deckpilot.projects`) is enabled, the only project context provider
    /// since the built-in `projects` module is gone (WP-C6; as the agent's
    /// `Agent::project_context_active`). Only then do plugins' project
    /// roles resolve.
    fn projects_enabled(&self) -> bool {
        PluginId::parse(crate::service::PROJECTS_PLUGIN).is_ok_and(|id| {
            self.manager
                .store()
                .plugin(&id)
                .ok()
                .flatten()
                .is_some_and(|record| record.state == PluginState::Enabled)
        })
    }

    /// The state of a key bound to a plugin action, for the engine's live
    /// key states (`key_state.rs`). Prepared like an action of the engine
    /// (enabled, granted, verdict, the declaration); asked only while the
    /// plugin's lane is idle, with [`STATE_CALL_TIMEOUT`] of wall clock
    /// and a run nobody started (of the key `scope`, which the plugin
    /// reads with `run.context`). The answer goes to `reply`; a failed
    /// state call is not written to the plugin's failure log (a key
    /// shows it), only a crash counts, in the supervisor.
    ///
    /// [`STATE_CALL_TIMEOUT`]: super::key_state::STATE_CALL_TIMEOUT
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the worker hands its message's parameters over; they are validated into new ones"
    )]
    pub fn key_state_for_engine(
        &mut self,
        plugin: &str,
        action: &str,
        params: Params,
        scope: Option<String>,
        abort: AbortSignal,
        reply: std::sync::mpsc::SyncSender<StateReply>,
    ) {
        let fail = |e: &IpcError| EngineFailure {
            code: engine_code(e.reason.as_deref()).to_owned(),
            message: e.message.clone(),
        };
        let prepared = plugin_id(plugin).and_then(|id| self.prepare(&id, action, true));
        let (record, prepared) = match prepared {
            Ok(prepared) => prepared,
            Err(e) => {
                let _ = reply.try_send(Err(fail(&e)));
                return;
            }
        };
        let params = match data::normalize_params(&prepared.definition, &params) {
            Ok(params) => params,
            Err(message) => {
                let _ = reply.try_send(Err(EngineFailure {
                    code: INVALID_PARAMETERS.to_owned(),
                    message,
                }));
                return;
            }
        };
        let done: StateDone = Box::new(move |result| {
            let _ = reply.try_send(result.map_err(|e| fail(&lane_error(&e))));
        });
        match self.lane(&record) {
            Ok(lane) => lane.submit_idle(Job {
                action: action.to_owned(),
                params,
                timeout: super::key_state::STATE_CALL_TIMEOUT,
                executor: prepared
                    .executor
                    .with_run(RunContext::unknown().with_key_scope(scope)),
                abort,
                reply: Reply::State(done),
            }),
            Err(e) => done(Err(LaneError::Refused(e))),
        }
    }

    /// A lane ended by itself. A crash loop or a panic disables the
    /// plugin until the user enables it again; a host with a weak sandbox
    /// only drops the lane (not a crash: the next invocation checks a
    /// fresh host).
    pub fn lane_ended(&mut self, plugin: &str, lane: u64, why: LaneEnd) {
        let Ok(id) = PluginId::parse(plugin) else {
            return;
        };
        if self.lanes.get(plugin).is_some_and(|l| l.id() == lane) {
            self.stop_host(&id);
        }
        match why {
            LaneEnd::WeakSandbox => {}
            LaneEnd::CrashLoop | LaneEnd::Panicked => {
                warn!(plugin, ?why, "the plugin is disabled");
                let _ = self.manager.disable(&id);
                self.sync_grants(&id);
                self.changed(&id);
            }
        }
    }
}

#[cfg(test)]
mod health_tests {
    use super::*;

    #[test]
    fn host_failures_are_counted_and_refusals_are_not() {
        let log = HealthLog::default();
        assert_eq!(log.get("acme.x"), None);
        for refusal in [LaneError::Busy, LaneError::Stopped, LaneError::Aborted] {
            log.record("acme.x", &refusal, 1);
        }
        assert_eq!(log.get("acme.x"), None);

        log.record(
            "acme.x",
            &LaneError::Supervisor(SupervisorError::BackingOff {
                retry_after: Duration::from_secs(3),
            }),
            2,
        );
        let health = log.get("acme.x").unwrap();
        assert_eq!(health.crashes, 0);
        assert_eq!(health.last_error_at, Some(2));
        assert_eq!(
            health.last_error.as_deref(),
            Some("the plugin host is restarting after a crash; try again in 3 s")
        );

        log.record(
            "acme.x",
            &LaneError::Supervisor(SupervisorError::Host(HostError::Timeout { status: None })),
            3,
        );
        log.record("acme.x", &LaneError::Panicked, 4);
        let health = log.get("acme.x").unwrap();
        assert_eq!(health.crashes, 1);
        assert_eq!(health.last_error_at, Some(4));
        assert_eq!(
            health.last_error.as_deref(),
            Some("the action failed inside DeckPilot; the plugin was disabled")
        );
        // Other plugins are not affected; uninstalling forgets.
        assert_eq!(log.get("acme.y"), None);
        log.forget("acme.x");
        assert_eq!(log.get("acme.x"), None);
    }

    #[test]
    fn a_watched_answer_is_still_delivered() {
        let log = HealthLog::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let done: Done = Box::new(move |result| {
            tx.send(result.is_err()).unwrap();
        });
        log.watch("acme.x".into(), done)(Err(LaneError::Panicked));
        assert!(rx.recv().unwrap());
        assert!(log.get("acme.x").is_some());
    }
}

/// One registry version as the app shows it: what it needs, and why this
/// DeckPilot (`host`) cannot run it, if it cannot.
fn available_version(
    v: &deckpilot_plugin_registry::DetailVersion,
    revoked: Option<String>,
    host: Option<&HostEnvironment>,
) -> ipc::AvailableVersion {
    let mut incompatible = Vec::new();
    if let Some(host) = host {
        if !v.engines.deckpilot.matches(&host.app_version) {
            incompatible.push(format!(
                "needs DeckPilot {}; this is DeckPilot {}",
                v.engines.deckpilot, host.app_version
            ));
        }
        if !v.engines.plugin_api.is_satisfied_by(host.plugin_api) {
            incompatible.push(format!(
                "needs plugin API {}; this DeckPilot provides {}",
                v.engines.plugin_api, host.plugin_api
            ));
        }
        if v.platforms
            .as_ref()
            .is_some_and(|p| !p.contains(&host.platform))
        {
            incompatible.push(format!("is not available for {}", host.platform));
        }
    }
    ipc::AvailableVersion {
        version: v.version.to_string(),
        permissions: v.permissions.clone(),
        withdrawn: v.yanked.as_ref().map(|y| y.reason.clone()).or(revoked),
        incompatible,
        deckpilot: Some(v.engines.deckpilot.to_string()),
        platforms: v
            .platforms
            .iter()
            .flatten()
            .map(|p| platform_id(*p).to_owned())
            .collect(),
        published_at: Some(v.published_at.timestamp()),
        dependencies: Vec::new(),
    }
}

/// The platform's id as manifests and the runtime status name it.
fn platform_id(platform: Platform) -> &'static str {
    match platform {
        Platform::Macos => "macos",
        Platform::Windows => "windows",
        Platform::Linux => "linux",
    }
}

/// Only the outputs the action declares leave the plugin.
fn filter_outputs(outcome: ActionOutcome, declared: &BTreeSet<String>) -> ActionOutcome {
    match outcome {
        ActionOutcome::Ok { message, data } => ActionOutcome::Ok {
            message,
            data: data
                .into_iter()
                .filter(|(key, _)| declared.contains(key))
                .collect(),
        },
        other => other,
    }
}

/// The action result code for an engine invocation that failed with
/// `reason`.
fn engine_code(why: Option<&str>) -> &'static str {
    match why {
        Some(reason::HOST_FAILED) => "plugin-host-failed",
        Some(reason::BUSY) => "plugin-busy",
        Some(reason::REVOKED) => "plugin-revoked",
        _ => "plugin-unavailable",
    }
}

/// Why an invocation that reached the lane failed, for the user.
fn lane_error(error: &LaneError) -> IpcError {
    match error {
        LaneError::Supervisor(e) => {
            self::error(ErrorCode::Failed, reason::HOST_FAILED, host_message(e))
        }
        LaneError::Refused(e) => e.clone(),
        LaneError::Busy => self::error(
            ErrorCode::Failed,
            reason::BUSY,
            format!(
                "the plugin is busy: {} actions are already waiting for it",
                super::lane::QUEUE
            ),
        ),
        LaneError::Stopped => self::error(
            ErrorCode::Failed,
            reason::NOT_ENABLED,
            "the plugin was stopped (disabled, updated or removed) before the action finished",
        ),
        LaneError::Aborted => self::error(
            ErrorCode::Failed,
            reason::HOST_FAILED,
            "the action was cancelled (or its plugin stopped); its result, if any, was discarded",
        ),
        LaneError::Panicked => self::error(
            ErrorCode::Failed,
            reason::HOST_FAILED,
            "the action failed inside DeckPilot; the plugin was disabled",
        ),
    }
}

/// Why an invocation failed, for the user.
fn host_message(error: &SupervisorError) -> String {
    match error {
        SupervisorError::Host(HostError::Crashed { fatal, status }) => match fatal {
            Some((why, detail)) => format!(
                "the plugin host stopped during the action ({why:?}: {detail}); it restarts with \
                 the next invocation"
            ),
            None => format!(
                "the plugin host stopped during the action ({status:?}); it restarts with the \
                 next invocation"
            ),
        },
        SupervisorError::Host(HostError::Timeout { .. }) => {
            "the action took longer than its time limit; the plugin host was stopped".to_owned()
        }
        SupervisorError::Host(HostError::Aborted { .. }) => {
            "the action was cancelled and did not finish within 2 s; the plugin host was stopped \
             (it starts again with the next action)"
                .to_owned()
        }
        SupervisorError::Host(HostError::SandboxMissing(report)) => sandbox_refusal(report),
        SupervisorError::Host(HostError::Spawn(e)) => {
            format!("the plugin host could not be started: {e}")
        }
        SupervisorError::Host(other) => format!("the plugin host failed: {other}"),
        SupervisorError::BackingOff { retry_after } => format!(
            "the plugin host is restarting after a crash; try again in {} s",
            retry_after.as_secs().max(1)
        ),
        SupervisorError::Disabled => {
            "the plugin crashed repeatedly and was disabled; enable it again to retry".to_owned()
        }
    }
}

/// The host's configuration for one plugin. Every host must report a
/// **full** OS sandbox (`Enforcement::Full`) in its `ready` message, on
/// every platform and on every (re)start: one that reports `partial` (Linux
/// without Landlock, Windows outside an `AppContainer`) or `none` is killed
/// before it receives the module.
fn host_config(
    host: &Path,
    module: Arc<[u8]>,
    actions: impl IntoIterator<Item = String>,
) -> HostConfig {
    let mut config = HostConfig::new(HostCommand::new(host), module).expect_actions(actions);
    config.require_sandbox = true;
    config.require_full_sandbox = true;
    config.shutdown_grace = Duration::from_secs(1);
    config
}

/// Why a host with a weaker sandbox than full was refused, for the user.
fn sandbox_refusal(report: &SandboxReport) -> String {
    let mechanism = if report.mechanism.is_empty() {
        "no mechanism"
    } else {
        report.mechanism.as_str()
    };
    let level = match report.enforcement {
        Enforcement::Full => "full",
        Enforcement::Partial => "partial",
        Enforcement::None => "none",
    };
    let mut message = format!(
        "the plugin host's sandbox is not full ({mechanism}, enforcement {level}); the host was \
         stopped and the plugin did not run: plugins run only in a full sandbox"
    );
    if !report.notes.is_empty() {
        message.push_str(" (");
        message.push_str(&report.notes.join("; "));
        message.push(')');
    }
    message
}

/// Reads a package file, refusing anything larger than a package can be.
fn read_package_file(path: &Path) -> Result<Vec<u8>, IpcError> {
    use std::io::Read as _;
    let limit = deckpilot_plugin_manifest::MAX_PACKAGE_BYTES;
    let file = std::fs::File::open(path).map_err(|e| {
        error(
            ErrorCode::NotFound,
            reason::INVALID_PACKAGE,
            format!("the package {} cannot be read: {e}", path.display()),
        )
    })?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| error(ErrorCode::Failed, reason::INVALID_PACKAGE, e.to_string()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(package_error(&PackageError::ArchiveTooLarge {
            size: limit + 1,
            limit,
        }));
    }
    Ok(bytes)
}

/// The response to give when the store cannot be opened.
pub(super) fn closed_status(problem: Option<&str>, unlocked: bool) -> PluginRuntimeStatus {
    PluginRuntimeStatus {
        available: false,
        built: true,
        enabled: true,
        execution_supported: super::execution_supported(),
        platform: platform().to_owned(),
        reason: Some(problem.map_or_else(
            || {
                if unlocked {
                    "the plugin store is opening".to_owned()
                } else {
                    "the configuration is locked; plugins are available once it is unlocked"
                        .to_owned()
                }
            },
            str::to_owned,
        )),
        registry: None,
        features: super::features(),
    }
}

pub(super) fn store_unavailable(problem: &str) -> IpcError {
    error(ErrorCode::Failed, reason::STORE_UNAVAILABLE, problem)
}

impl Drop for PluginService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Whether the module bytes about to be compiled are the installed
/// version's `plugin.wasm` as its integrity record holds it.
fn module_matches_record(
    integrity: &deckpilot_plugin_manifest::PackageIntegrity,
    module: &[u8],
) -> Result<(), String> {
    integrity
        .verify_file(
            deckpilot_plugin_package::ARTIFACT,
            &deckpilot_plugin_package::sha256(module),
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_recorded_module_bytes_are_compiled() {
        let module = b"\0asm\x01\0\0\0";
        let digest = deckpilot_plugin_package::sha256(module);
        let integrity = deckpilot_plugin_manifest::PackageIntegrity {
            manifest_sha256: digest,
            package: deckpilot_plugin_manifest::PackageDigest {
                sha256: digest,
                size: 1,
            },
            files: [(deckpilot_plugin_package::ARTIFACT.to_owned(), digest)].into(),
        };
        module_matches_record(&integrity, module).unwrap();
        // A module swapped after `verify` read the directory is refused.
        let why = module_matches_record(&integrity, b"\0asm\x01\0\0\0\x01").unwrap_err();
        assert!(why.contains("plugin.wasm was modified"), "{why}");
        let none = deckpilot_plugin_manifest::PackageIntegrity {
            files: BTreeMap::new(),
            ..integrity
        };
        assert!(module_matches_record(&none, module).is_err());
    }

    #[test]
    fn errors_carry_reasons() {
        let id = PluginId::parse("acme.tool").unwrap();
        assert_eq!(
            store_error(&StoreError::NotInstalled(id)).reason.as_deref(),
            Some(reason::NOT_INSTALLED)
        );
        let js = PackageError::UnsupportedRuntime {
            found: "plugin.js".into(),
        };
        assert_eq!(
            package_error(&js).reason.as_deref(),
            Some(reason::UNSUPPORTED_RUNTIME)
        );
    }

    #[test]
    fn every_host_must_report_a_full_sandbox() {
        let config = host_config(
            Path::new("host"),
            Arc::from(&b"\0asm"[..]),
            ["greet".to_owned()],
        );
        assert!(config.require_sandbox && config.require_full_sandbox);
        assert!(config.command.args.is_empty(), "no --no-os-sandbox");
    }

    #[test]
    fn a_weak_sandbox_is_explained() {
        let report = SandboxReport {
            mechanism: "windows-job-lowil".into(),
            enforcement: Enforcement::Partial,
            notes: vec!["not launched in an AppContainer".into()],
        };
        let message = host_message(&SupervisorError::Host(HostError::SandboxMissing(report)));
        assert!(message.contains("windows-job-lowil"), "{message}");
        assert!(message.contains("partial"), "{message}");
        assert!(message.contains("did not run"), "{message}");
        assert!(message.contains("AppContainer"), "{message}");
    }

    #[test]
    fn the_host_is_looked_up_next_to_the_agent() {
        let configured = PluginSettings {
            host: Some(PathBuf::from("/opt/host")),
            ..PluginSettings::default()
        };
        assert_eq!(host_program(&configured), PathBuf::from("/opt/host"));
        let default = host_program(&PluginSettings::default());
        assert!(
            default
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("deckpilot-plugin-host")
        );
    }
}
