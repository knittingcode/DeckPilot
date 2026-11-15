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
use deckpilot_ipc::plugins::PluginEvent;
use deckpilot_ipc::plugins::{
    self as ipc, PluginInstallSource, PluginOperation, PluginRuntimeStatus, PluginStage,
    PluginUpdateOutcome, reason,
};
use deckpilot_ipc::{ErrorCode, IpcError, Request, Response};
use deckpilot_plugin_abi::protocol::{Enforcement, SandboxReport};
use deckpilot_plugin_abi::{ActionOutcome, Params};
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

use super::convert;
use super::data;
use super::engine::CatalogPlugin;
use super::executor::{DataAccess, Executor, SharedData, Transport};
use super::lane::{Done, Ended, Job, Lane, LaneEnd, LaneError};
use super::registry::LocalRegistry;
use super::worker::{EngineFailure, EngineReply, Events};
use super::{PluginSettings, platform, platform_problem};

/// Who decides in the audit log.
const DECIDED_BY: &str = "user";
/// The shared code of an invocation whose parameters do not match the
/// action's declaration (refused by the agent, before the host).
const INVALID_PARAMETERS: &str = "invalid-parameters";
/// The plugin API level this agent provides (`deckpilot-plugin-sdk` 1.0).
const PLUGIN_API: ApiLevel = ApiLevel::new(1, 0);
/// Longest any invocation may take, whatever the manifest says.
const MAX_INVOCATION: Duration = Duration::from_secs(600);
/// Where the registry's verified state is kept (outside `plugins/`,
/// whose unknown entries the store's recovery sweeps away).
const REGISTRY_STATE_DIR: &str = "plugin-registry";

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
        E::ConsentRequired { .. } => {
            self::error(ErrorCode::Failed, reason::CONSENT_REQUIRED, message)
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
        | E::NotNewer { .. } => self::error(ErrorCode::Failed, reason::INVALID_STATE, message),
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
    registry: Option<LocalRegistry>,
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
    /// The capability executors' connection to plugins.db (storage,
    /// secrets, settings); emptied when the service shuts down.
    data: SharedData,
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
    ) -> Result<Self, StoreError> {
        let mut manager = PluginManager::open(data_dir, key, manager_options())?;
        let report = manager.recover()?;
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
        let (registry, registry_problem) = match (&settings.registry, &settings.registry_roots) {
            (Some(dir), Some(roots)) => {
                match LocalRegistry::open(dir, roots, &data_dir.join(REGISTRY_STATE_DIR)) {
                    Ok(registry) => (Some(registry), None),
                    Err(problem) => (None, Some(problem)),
                }
            }
            (Some(_), None) => (
                None,
                Some("the plugin registry has no root keys (--plugin-registry-roots)".to_owned()),
            ),
            _ => (None, Some("no plugin registry is configured".to_owned())),
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
            data: Arc::new(Mutex::new(Some(data))),
        };
        if service.registry.is_some() {
            let _ = service.refresh();
            service.apply_verdicts();
        }
        for record in service.manager.store().plugins()? {
            service.sync_grants(&record.id);
        }
        Ok(service)
    }

    /// Stops every host.
    pub fn shutdown(&mut self) {
        for (_, lane) in std::mem::take(&mut self.lanes) {
            lane.stop();
        }
        // Closes the executors' connection, so nothing keeps plugins.db
        // open after a lock (or a rekey).
        if let Ok(mut data) = self.data.lock() {
            *data = None;
        }
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
                name: known.display_name,
                version: known.last_version,
                description: String::new(),
                actions: known.actions,
                unavailable: Some("the plugin is not installed".to_owned()),
            });
        records
            .iter()
            .filter_map(|record| {
                let manifest = &record.active()?.manifest;
                let grants = store.grants(&record.id).unwrap_or_default();
                Some(CatalogPlugin {
                    id: record.id.to_string(),
                    name: manifest.name.clone(),
                    version: manifest.version.to_string(),
                    description: manifest.description.clone(),
                    actions: manifest.contributes.actions().to_vec(),
                    unavailable: convert::unavailable_with(record, &grants),
                })
            })
            .chain(gone)
            .collect()
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
            registry: self.registry.as_ref().map(LocalRegistry::status),
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
    }

    fn record(&self, id: &PluginId) -> Result<PluginRecord, IpcError> {
        self.manager
            .store()
            .plugin(id)
            .map_err(|e| store_error(&e))?
            .ok_or_else(|| store_error(&StoreError::NotInstalled(id.clone())))
    }

    fn details(&self, id: &PluginId) -> Result<Response, IpcError> {
        let record = self.record(id)?;
        let store = self.manager.store();
        let grants = store.grants(id).map_err(|e| store_error(&e))?;
        let audit = store.audit(id).map_err(|e| store_error(&e))?;
        let pending = self
            .manager
            .pending_update(id)
            .map_err(|e| store_error(&e))?;
        Ok(Response::Plugin {
            plugin: Box::new(convert::details(&record, &grants, &audit, pending)),
        })
    }

    fn registry(&self) -> Result<&LocalRegistry, IpcError> {
        self.registry.as_ref().ok_or_else(|| {
            error(
                ErrorCode::Failed,
                reason::REGISTRY,
                self.registry_problem
                    .clone()
                    .unwrap_or_else(|| "no plugin registry is configured".into()),
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
        let Some(registry) = self.registry.as_ref() else {
            return;
        };
        match self.manager.apply_verdicts(registry.client()) {
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

    /// Registry pins for `id` `version` and the package bytes they cover.
    fn fetch(
        &self,
        id: &PluginId,
        version: &Version,
    ) -> Result<(ExpectedPackage, Vec<u8>), IpcError> {
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
        let bytes = registry
            .package(&target.url)
            .map_err(|m| error(ErrorCode::Failed, reason::REGISTRY, m))?;
        Ok((target.expected, bytes))
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
        let (approved, skipped) = convert::approved(&effective);
        if !skipped.is_empty() {
            info!(plugin = %id, ?skipped, "grants the broker does not enforce are not passed on");
        }
        if let Err(e) = self.broker.set_grants(&broker_id, &approved) {
            warn!(plugin = %id, error = %e, "the broker refused the grants; the plugin gets none");
            self.broker.remove_plugin(&broker_id);
        }
    }

    /// Stops the plugin's lane: a running invocation is aborted, queued
    /// ones are refused, the host is stopped.
    fn stop_host(&mut self, id: &PluginId) {
        if let Some(lane) = self.lanes.remove(id.as_str()) {
            lane.stop();
        }
    }

    /// The registry's verdict on the installed version; a `disable`
    /// quarantines the plugin.
    fn check_verdict(&mut self, record: &PluginRecord) -> Result<(), IpcError> {
        let (Some(registry), Some(active)) = (self.registry.as_ref(), record.active()) else {
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
            Request::ListPlugins => {
                let plugins = self
                    .manager
                    .store()
                    .plugins()
                    .map_err(|e| store_error(&e))?;
                let store = self.manager.store();
                Ok(Response::Plugins {
                    plugins: plugins
                        .iter()
                        .map(|record| {
                            convert::summary(record, &store.grants(&record.id).unwrap_or_default())
                        })
                        .collect(),
                })
            }
            Request::GetPlugin { plugin } => self.details(&plugin_id(&plugin)?),
            Request::ListAvailablePlugins { refresh } => self.available(refresh),
            Request::InstallPlugin { source, consent } => self.install(&source, &consent),
            Request::EnablePlugin { plugin, consent } => {
                self.enable(&plugin_id(&plugin)?, &consent)
            }
            Request::DisablePlugin { plugin } => self.disable(&plugin_id(&plugin)?),
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
            Request::UninstallPlugin { plugin, keep } => self.uninstall(
                &plugin_id(&plugin)?,
                Retention {
                    settings: keep.settings,
                    credentials: keep.credentials,
                },
            ),
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
                        .map(|k| data::known(k, installed.iter().any(|r| r.id == k.id)))
                        .collect(),
                })
            }
            Request::GetPluginSettings { plugin } => self.settings(&plugin_id(&plugin)?),
            Request::SetPluginSettings { plugin, values } => {
                let id = plugin_id(&plugin)?;
                let manifest = self.active_manifest(&id)?;
                let saved = self
                    .manager
                    .store()
                    .settings(&id)
                    .map_err(|e| store_error(&e))?;
                let merged = data::merge_settings(&manifest, &saved, &values)
                    .map_err(|e| error(ErrorCode::InvalidRequest, reason::INVALID_DATA, e))?;
                self.manager
                    .set_settings(&id, &merged)
                    .map_err(|e| store_error(&e))?;
                info!(plugin = %id, "plugin settings saved");
                self.settings(&id)
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
                        let mut incompatible = Vec::new();
                        if let Some(host) = &host {
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
                                incompatible
                                    .push(format!("is not available for {}", host.platform));
                            }
                        }
                        ipc::AvailableVersion {
                            version: v.version.to_string(),
                            permissions: v.permissions.clone(),
                            withdrawn: v.yanked.as_ref().map(|y| y.reason.clone()).or(revoked),
                            incompatible,
                        }
                    })
                    .collect();
                versions.reverse();
                plugins.push(ipc::AvailablePlugin {
                    id: entry.id.to_string(),
                    name: entry.name.clone(),
                    summary: entry.summary.clone(),
                    publisher: entry.publisher.clone(),
                    latest: entry.latest.stable.as_ref().map(ToString::to_string),
                    versions,
                    installed: installed.get(entry.id.as_str()).cloned(),
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
                let (expected, _) = self.fetch(&manifest.id, &manifest.version)?;
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
        self.manager.disable(id).map_err(|e| store_error(&e))?;
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
            let (expected, bytes) = self.fetch(id, &version)?;
            self.progress(id.as_str(), PluginOperation::Update, PluginStage::Verifying);
            let verified = verify_package(&bytes, &expected, &Limits::default())
                .map_err(|e| package_error(&e))?;
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
                    changes: convert::changes(&diff, &self.active_manifest(id)?),
                }
            }
            UpdateOutcome::AwaitingConsent { version, diff } => {
                let escalations: BTreeSet<GrantKey> = diff.escalations().into_iter().collect();
                if !escalations.is_empty() && escalations.is_subset(&consent.keys) {
                    let changes = convert::changes(&diff, &self.staged_manifest(id)?);
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
                        changes: convert::changes(&diff, &self.staged_manifest(id)?),
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

    fn approve(&mut self, id: &PluginId, consent: &[String]) -> Result<Response, IpcError> {
        let consent = Self::consent(consent)?;
        let (_, diff) = self
            .manager
            .pending_update(id)
            .map_err(|e| store_error(&e))?
            .ok_or_else(|| store_error(&StoreError::NoStagedUpdate(id.clone())))?;
        let changes = convert::changes(&diff, &self.staged_manifest(id)?);
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
        self.stop_host(id);
        let result = match self.registry.as_ref() {
            Some(registry) => self.manager.rollback(id, registry.client()),
            None => self.manager.rollback(id, &NoRevocations),
        };
        result.map_err(|e| store_error(&e))?;
        self.after_version_change(id);
        self.changed(id);
        self.details(id)
    }

    fn settings(&self, id: &PluginId) -> Result<Response, IpcError> {
        let manifest = self.active_manifest(id)?;
        let saved = self
            .manager
            .store()
            .settings(id)
            .map_err(|e| store_error(&e))?;
        Ok(Response::PluginSettings {
            settings: Box::new(data::settings(&manifest, saved)),
        })
    }

    fn credentials(&self, id: &PluginId) -> Result<Response, IpcError> {
        let manifest = self.active_manifest(id)?;
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
        self.manager
            .uninstall_keeping(id, keep)
            .map_err(|e| store_error(&e))?;
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

    /// The running module changed: its host is restarted on demand.
    fn after_version_change(&mut self, id: &PluginId) {
        self.stop_host(id);
        self.sync_grants(id);
    }

    fn active_manifest(&self, id: &PluginId) -> Result<Manifest, IpcError> {
        self.record(id)?
            .active()
            .map(|v| v.manifest.clone())
            .ok_or_else(|| store_error(&StoreError::NotInstalled(id.clone())))
    }

    fn staged_manifest(&self, id: &PluginId) -> Result<Manifest, IpcError> {
        self.record(id)?
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
            let path = self
                .manager
                .layout()
                .resolve(&active.install_dir)
                .join(deckpilot_plugin_package::ARTIFACT);
            let module = std::fs::read(&path).map_err(|e| {
                error(
                    ErrorCode::Failed,
                    reason::BROKEN,
                    format!("the plugin module cannot be read: {e}"),
                )
            })?;
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
        let node = declaration.node.clone().unwrap_or_default();
        let timeout = Duration::from_millis(node.timeout_ms()).min(MAX_INVOCATION);
        let outputs = super::lane::declared_outputs(&declaration);
        let name = active.manifest.name.clone();
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
        self.sync_grants(id);
        let executor = Executor::new(
            self.broker.clone(),
            name,
            self.events.agent.clone(),
            self.transport.clone(),
        )
        .with_data(access);
        Ok((
            record,
            Prepared {
                timeout,
                outputs,
                executor,
                definition,
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
        } = prepared;
        // Checked against the manifest before the host sees anything.
        let params = match data::normalize_params(&definition, &params) {
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
        match self.lane(&record) {
            Ok(lane) => lane.submit(Job {
                action: action.to_owned(),
                params,
                timeout,
                executor,
                abort: AbortSignal::new(),
                done,
            }),
            Err(e) => done(Err(LaneError::Refused(e))),
        }
    }

    /// A plugin action from the workflow engine (a key, `runAction`, a
    /// workflow node). The answer goes to `reply`; a caller that gave up
    /// has dropped its end, and the answer is discarded.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the worker hands its message's parameters over; they are validated into new ones"
    )]
    pub fn invoke_for_engine(
        &mut self,
        plugin: &str,
        action: &str,
        params: Params,
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
        } = prepared;
        let params = match data::normalize_params(&definition, &params) {
            Ok(params) => params,
            Err(message) => {
                let _ = reply.try_send(Err(EngineFailure {
                    code: INVALID_PARAMETERS.to_owned(),
                    message,
                }));
                return;
            }
        };
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
        match self.lane(&record) {
            Ok(lane) => lane.submit(Job {
                action: action.to_owned(),
                params,
                timeout,
                executor,
                abort,
                done,
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
            "the action was cancelled before it started",
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

#[cfg(test)]
mod tests {
    use super::*;

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
