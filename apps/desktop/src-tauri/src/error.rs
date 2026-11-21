//! The error of every command, as the frontend receives it.
//!
//! A [`CommandError`] is serialized as `{ code, message, params? }`
//! (docs/i18n.md, "Backend errors"):
//!
//! - `code` is stable and machine-readable, derived from the variant and the
//!   kind of the inner error (`locked`, `storage.invalidKey`,
//!   `agent.notRunning`, `invalid.layoutNameEmpty`, …). The frontend
//!   translates it with `errors:<code>`.
//! - `message` is the English text ([`Display`](std::fmt::Display)), for
//!   logs and as the fallback when the frontend has no translation.
//! - `params` are the values the translation interpolates (`{{count}}`).

use std::collections::BTreeMap;

use deckpilot_core::CatalogError;
use deckpilot_core::registry::RegistryError;
use deckpilot_core::{EngineError, ModuleError, ProfileIssue, project::ProjectContextError};
use deckpilot_ipc::{ClientError, ErrorCode, IpcError};
use deckpilot_storage::{StorageError, crypto::CryptoError};
use serde::{Serialize, Serializer};

/// Error returned to the frontend. Serialized as `{ code, message, params? }`.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Module(#[from] ModuleError),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Profile(#[from] ProfileIssue),
    #[error(transparent)]
    ProjectContext(#[from] ProjectContextError),
    #[error(transparent)]
    Identifier(#[from] deckpilot_core::IdentifierError),
    #[error(transparent)]
    Agent(#[from] ClientError),
    #[cfg(feature = "self-update")]
    #[error(transparent)]
    Updater(#[from] tauri_plugin_updater::Error),
    #[error("the configuration is locked")]
    Locked,
    #[error("confirm that it is you to continue")]
    ConfirmationRequired,
    /// The operation is sensitive; the frontend asks the user to
    /// authenticate and retries. The code is matched in `src/lib/api.ts`.
    #[error("authentication required")]
    AuthenticationRequired,
    /// A refusal without a code of its own (code `invalid`): the frontend
    /// shows the message as it is. Prefer [`CommandError::invalid`] for a
    /// sentence the user sees.
    #[error("{0}")]
    Invalid(String),
    /// A refusal with a stable reason (code `invalid.<reason>`).
    #[error("{message}")]
    Rejected {
        reason: &'static str,
        message: String,
        params: Params,
    },
    #[error("{0} not found")]
    NotFound(&'static str),
}

/// A value a translation interpolates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Param {
    Text(String),
    Number(u64),
}

impl From<&str> for Param {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for Param {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<u32> for Param {
    fn from(value: u32) -> Self {
        Self::Number(value.into())
    }
}

impl From<u64> for Param {
    fn from(value: u64) -> Self {
        Self::Number(value)
    }
}

impl From<usize> for Param {
    fn from(value: usize) -> Self {
        Self::Number(u64::try_from(value).unwrap_or(u64::MAX))
    }
}

/// The named values of an error, for its translation.
pub type Params = BTreeMap<&'static str, Param>;

fn params<const N: usize>(entries: [(&'static str, Param); N]) -> Params {
    entries.into_iter().collect()
}

fn detail(value: &(impl ToString + ?Sized)) -> Params {
    params([("detail", Param::Text(value.to_string()))])
}

/// `"device model"` → `"deviceModel"`: the code of a thing's name.
fn camel(words: &str) -> String {
    let mut out = String::with_capacity(words.len());
    for (index, word) in words.split_whitespace().enumerate() {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            if index == 0 {
                out.push(first);
            } else {
                out.extend(first.to_uppercase());
            }
            out.extend(chars);
        }
    }
    out
}

/// A list of names as one parameter: `“Work”, “Home”`.
fn quoted<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    names
        .into_iter()
        .map(|name| format!("“{name}”"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl CommandError {
    /// A refusal the frontend translates as `errors:invalid.<reason>`;
    /// `message` is the English text.
    pub fn invalid(reason: &'static str, message: impl Into<String>) -> Self {
        Self::invalid_with(reason, message, [])
    }

    /// [`CommandError::invalid`] with the values its translation uses.
    pub fn invalid_with<const N: usize>(
        reason: &'static str,
        message: impl Into<String>,
        values: [(&'static str, Param); N],
    ) -> Self {
        Self::Rejected {
            reason,
            message: message.into(),
            params: params(values),
        }
    }

    /// The stable code and the parameters of its translation.
    pub fn code(&self) -> (String, Params) {
        let fixed = |code: &str| (code.to_owned(), Params::new());
        match self {
            Self::Storage(error) => storage_code(error),
            Self::Registry(error) => registry_code(error),
            Self::Engine(error) => engine_code(error),
            Self::Module(error) => module_code(error),
            Self::Catalog(error) => catalog_code(error),
            Self::Profile(error) => profile_code(error),
            Self::ProjectContext(error) => project_context_code(error),
            Self::Identifier(error) => (
                "identifier".to_owned(),
                params([
                    ("kind", error.kind.into()),
                    ("value", error.value.clone().into()),
                    ("reason", error.reason.into()),
                ]),
            ),
            Self::Agent(error) => agent_code(error),
            #[cfg(feature = "self-update")]
            Self::Updater(_) => fixed("updater"),
            Self::Locked => fixed("locked"),
            Self::ConfirmationRequired => fixed("confirmationRequired"),
            Self::AuthenticationRequired => fixed("authenticationRequired"),
            Self::Invalid(_) => fixed("invalid"),
            Self::Rejected { reason, params, .. } => (format!("invalid.{reason}"), params.clone()),
            Self::NotFound(what) => (format!("notFound.{}", camel(what)), Params::new()),
        }
    }
}

fn storage_code(error: &StorageError) -> (String, Params) {
    use StorageError as E;
    let (kind, values): (&str, Params) = match error {
        E::Database(e) => ("database", detail(e)),
        E::Document(e) => ("document", detail(e)),
        E::UnsupportedSchema { found, supported } => (
            "unsupportedSchema",
            params([
                ("found", (*found).into()),
                ("supported", (*supported).into()),
            ]),
        ),
        E::UnsupportedDocument(version) => (
            "unsupportedDocument",
            params([("version", (*version).into())]),
        ),
        E::EncryptionUnavailable => ("encryptionUnavailable", Params::new()),
        E::SqlCipherIncomplete(reason) => ("sqlCipherIncomplete", detail(reason)),
        E::InvalidKey => ("invalidKey", Params::new()),
        E::UnknownProfile => ("unknownProfile", Params::new()),
        E::UnknownLayout => ("unknownLayout", Params::new()),
        E::LayoutInUse(users) => {
            let profiles = if users.is_empty() {
                None
            } else {
                Some(quoted(users.iter().map(|(_, name)| name.as_str())))
            };
            return match profiles {
                Some(profiles) => (
                    "storage.layoutInUse".to_owned(),
                    params([("profiles", profiles.into())]),
                ),
                None => ("storage.layoutInUseNone".to_owned(), Params::new()),
            };
        }
        E::Io(e) => ("io", detail(e)),
        E::Corrupt(reason) => ("corrupt", detail(reason)),
        E::MigrationFailed { reason, backup } => (
            "migrationFailed",
            params([
                ("reason", reason.clone().into()),
                ("backup", backup.as_deref().unwrap_or("(none)").into()),
            ]),
        ),
        E::UnsupportedModuleSchema {
            module,
            found,
            supported,
        } => (
            "unsupportedModuleSchema",
            params([
                ("module", module.clone().into()),
                ("found", (*found).into()),
                ("supported", (*supported).into()),
            ]),
        ),
        E::Crypto(e) => {
            return (
                format!("storage.crypto.{}", crypto_kind(e)),
                crypto_params(e),
            );
        }
        E::Vault(_) => ("vault", Params::new()),
        E::Invalid(_) => ("invalid", Params::new()),
        E::KeyStore(_) => ("keyStore", Params::new()),
        E::DatabaseMissing => ("databaseMissing", Params::new()),
        E::CredentialsRequired(what) => {
            return (
                format!("storage.credentialsRequired.{}", camel(what)),
                Params::new(),
            );
        }
        E::ProjectOfAnotherProfile(id) => (
            "projectOfAnotherProfile",
            params([("project", id.to_string().into())]),
        ),
        E::DuplicateProjectKey(key) => {
            ("duplicateProjectKey", params([("key", key.clone().into())]))
        }
        E::UnknownVariable(key) => ("unknownVariable", params([("key", key.clone().into())])),
        E::VariableKeyTaken { key, module } => (
            "variableKeyTaken",
            params([
                ("key", key.clone().into()),
                ("module", module.clone().into()),
            ]),
        ),
        E::VariableValuesConflict { key, count } => (
            "variableValuesConflict",
            params([("key", key.clone().into()), ("count", (*count).into())]),
        ),
        E::Variable(e) => ("variable", detail(e)),
        E::WorkSessionActive => ("workSessionActive", Params::new()),
        E::VariableInUse(uses) => ("variableInUse", params([("count", uses.len().into())])),
        E::ActionInUse(uses) => ("actionInUse", params([("count", uses.len().into())])),
    };
    (format!("storage.{kind}"), values)
}

fn crypto_kind(error: &CryptoError) -> &'static str {
    match error {
        CryptoError::PasswordTooShort => "passwordTooShort",
        CryptoError::PasswordTooLong => "passwordTooLong",
        CryptoError::ConfirmationMismatch => "confirmationMismatch",
        CryptoError::UnsupportedKdf(_) => "unsupportedKdf",
        CryptoError::Derivation => "derivation",
        CryptoError::Random => "random",
        CryptoError::Encoding => "encoding",
    }
}

fn crypto_params(error: &CryptoError) -> Params {
    match error {
        CryptoError::PasswordTooShort => params([(
            "count",
            deckpilot_storage::crypto::MIN_PASSWORD_CHARS.into(),
        )]),
        CryptoError::UnsupportedKdf(reason) => detail(reason),
        _ => Params::new(),
    }
}

fn registry_code(error: &RegistryError) -> (String, Params) {
    let (kind, values) = match error {
        RegistryError::Duplicate(id) => {
            ("duplicate", params([("actionType", id.to_string().into())]))
        }
        RegistryError::UnknownActionType(id) => (
            "unknownActionType",
            params([("actionType", id.to_string().into())]),
        ),
        RegistryError::Unavailable(id, reason) => (
            "unavailable",
            params([
                ("actionType", id.to_string().into()),
                ("reason", reason.clone().into()),
            ]),
        ),
        RegistryError::EmptyName => ("emptyName", Params::new()),
        RegistryError::InvalidParameters(problems) => (
            "invalidParameters",
            detail(
                &problems
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        ),
        RegistryError::InvalidPresentation(_) => ("invalidPresentation", Params::new()),
    };
    (format!("registry.{kind}"), values)
}

fn engine_code(error: &EngineError) -> (String, Params) {
    let (kind, values) = match error {
        EngineError::UnknownAction(_) => ("unknownAction", Params::new()),
        EngineError::UnknownWorkflow(_) => ("unknownWorkflow", Params::new()),
        EngineError::UnknownProject(_) => ("unknownProject", Params::new()),
        EngineError::NoActiveProfile => ("noActiveProfile", Params::new()),
        EngineError::Context(error) => return project_context_code(error),
        EngineError::InvalidWorkflow(reason) => ("invalidWorkflow", detail(reason)),
    };
    (format!("engine.{kind}"), values)
}

fn module_code(error: &ModuleError) -> (String, Params) {
    let module = |id: &dyn ToString| ("module", Param::Text(id.to_string()));
    let (kind, values) = match error {
        ModuleError::Duplicate(id) => ("duplicate", params([module(id)])),
        ModuleError::Reserved(id) => ("reserved", params([module(id)])),
        ModuleError::Unknown(id) => ("unknown", params([module(id)])),
        ModuleError::Namespace {
            module: id,
            action_type,
        } => (
            "namespace",
            params([module(id), ("actionType", action_type.to_string().into())]),
        ),
        ModuleError::DuplicateActionType(id) => (
            "duplicateActionType",
            params([("actionType", id.to_string().into())]),
        ),
        ModuleError::MissingDependency {
            module: id,
            dependency,
        } => (
            "missingDependency",
            params([module(id), ("dependency", dependency.to_string().into())]),
        ),
        ModuleError::Required {
            module: id,
            dependent,
        } => (
            "required",
            params([module(id), ("dependent", dependent.to_string().into())]),
        ),
        ModuleError::Unsupported(id) => ("unsupported", params([module(id)])),
        ModuleError::WindowNamespace { module: id, window } => (
            "windowNamespace",
            params([module(id), ("window", window.clone().into())]),
        ),
        ModuleError::Variables(e) => ("variables", detail(e)),
        ModuleError::InvalidConfig(problems) => (
            "invalidConfig",
            detail(
                &problems
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        ),
        ModuleError::IncompatibleExport {
            module: id,
            found,
            supported,
        } => (
            "incompatibleExport",
            params([
                module(id),
                ("found", (*found).into()),
                ("supported", (*supported).into()),
            ]),
        ),
    };
    (format!("module.{kind}"), values)
}

fn catalog_code(error: &CatalogError) -> (String, Params) {
    let (kind, values) = match error {
        CatalogError::DuplicateKey(key) => ("duplicateKey", params([("key", key.clone().into())])),
        CatalogError::NotFound => ("notFound", Params::new()),
        CatalogError::InUse(count) => ("inUse", params([("count", (*count).into())])),
        CatalogError::IdInUse => ("idInUse", Params::new()),
        CatalogError::SameProfile => ("sameProfile", Params::new()),
    };
    (format!("catalog.{kind}"), values)
}

fn profile_code(error: &ProfileIssue) -> (String, Params) {
    let (kind, values) = match error {
        ProfileIssue::EmptyName => ("emptyName", Params::new()),
        ProfileIssue::DuplicateProjectKey(key) => {
            ("duplicateProjectKey", params([("key", key.clone().into())]))
        }
        ProfileIssue::MissingWorkflowAction { workflow, .. } => (
            "missingWorkflowAction",
            params([("workflow", workflow.clone().into())]),
        ),
        ProfileIssue::Layout { layout, issue } => (
            "layout",
            params([
                ("layout", layout.clone().into()),
                ("detail", issue.to_string().into()),
            ]),
        ),
        ProfileIssue::InUse(count) => ("inUse", params([("count", (*count).into())])),
        ProfileIssue::NotFound(what) => {
            return (format!("profile.notFound.{}", camel(what)), Params::new());
        }
        ProfileIssue::ModelMismatch => ("modelMismatch", Params::new()),
        ProfileIssue::ForeignProject(name) => {
            ("foreignProject", params([("name", name.clone().into())]))
        }
        ProfileIssue::ForeignDefaultProject => ("foreignDefaultProject", Params::new()),
        ProfileIssue::InvalidIcon(_) => ("invalidIcon", Params::new()),
        ProfileIssue::TooManyIcons => (
            "tooManyIcons",
            params([("max", deckpilot_core::MAX_ICONS.into())]),
        ),
        ProfileIssue::HomePage => ("homePage", Params::new()),
        ProfileIssue::PageCycle => ("pageCycle", Params::new()),
        ProfileIssue::PageOrder => ("pageOrder", Params::new()),
        ProfileIssue::OpensOutside => ("opensOutside", Params::new()),
        ProfileIssue::IncompatiblePage(reason) => ("incompatiblePage", detail(reason)),
    };
    (format!("profile.{kind}"), values)
}

fn project_context_code(error: &ProjectContextError) -> (String, Params) {
    let kind = match error {
        ProjectContextError::ProfileMismatch => "profileMismatch",
        ProjectContextError::UnknownProject => "unknownProject",
    };
    (format!("projectContext.{kind}"), Params::new())
}

fn agent_code(error: &ClientError) -> (String, Params) {
    match error {
        ClientError::NotRunning => ("agent.notRunning".to_owned(), Params::new()),
        ClientError::Closed => ("agent.closed".to_owned(), Params::new()),
        ClientError::Io(e) => ("agent.io".to_owned(), detail(e)),
        ClientError::Protocol(reason) => ("agent.protocol".to_owned(), detail(reason)),
        ClientError::Agent(error) => ipc_code(error),
        ClientError::Unsupported {
            request,
            needs,
            protocol,
            agent,
        } => (
            "agent.tooOld".to_owned(),
            params([
                ("agent", agent.clone().into()),
                ("request", request.clone().into()),
                ("needs", (*needs).into()),
                ("protocol", (*protocol).into()),
            ]),
        ),
    }
}

/// An error the agent answered with: `agent.<ErrorCode>`, with the agent's
/// machine-readable `reason` (if any) as a parameter.
fn ipc_code(error: &IpcError) -> (String, Params) {
    let code = match error.code {
        ErrorCode::InvalidRequest => "invalidRequest",
        ErrorCode::Unauthorized => "unauthorized",
        ErrorCode::UnsupportedProtocol => "unsupportedProtocol",
        ErrorCode::NotFound => "notFound",
        ErrorCode::Unsupported => "unsupported",
        ErrorCode::Failed => "failed",
        ErrorCode::Locked => "locked",
    };
    let values = error
        .reason
        .as_ref()
        .map(|reason| params([("reason", reason.clone().into())]))
        .unwrap_or_default();
    (format!("agent.{code}"), values)
}

#[derive(Serialize)]
struct Payload {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    params: Params,
}

impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (code, params) = self.code();
        Payload {
            code,
            message: self.to_string(),
            params,
        }
        .serialize(serializer)
    }
}

pub type CommandResult<T> = Result<T, CommandError>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(error: &CommandError) -> serde_json::Value {
        serde_json::to_value(error).unwrap()
    }

    #[test]
    fn plain_variants_have_a_code_and_their_message() {
        assert_eq!(
            payload(&CommandError::Locked),
            json!({ "code": "locked", "message": "the configuration is locked" })
        );
        assert_eq!(
            payload(&CommandError::ConfirmationRequired),
            json!({
                "code": "confirmationRequired",
                "message": "confirm that it is you to continue"
            })
        );
        assert_eq!(
            payload(&CommandError::AuthenticationRequired),
            json!({ "code": "authenticationRequired", "message": "authentication required" })
        );
        assert_eq!(
            payload(&CommandError::NotFound("device model")),
            json!({ "code": "notFound.deviceModel", "message": "device model not found" })
        );
    }

    #[test]
    fn refusals_carry_their_reason_and_params() {
        assert_eq!(
            payload(&CommandError::Invalid("something internal".into())),
            json!({ "code": "invalid", "message": "something internal" })
        );
        assert_eq!(
            payload(&CommandError::invalid(
                "passwordIncorrect",
                "the password is not correct"
            )),
            json!({ "code": "invalid.passwordIncorrect", "message": "the password is not correct" })
        );
        assert_eq!(
            payload(&CommandError::invalid_with(
                "layoutUsedElsewhere",
                "the layout is also used by “Home”",
                [("profiles", "“Home”".into())],
            )),
            json!({
                "code": "invalid.layoutUsedElsewhere",
                "message": "the layout is also used by “Home”",
                "params": { "profiles": "“Home”" }
            })
        );
        assert_eq!(
            payload(&CommandError::invalid_with(
                "workflowsRunning",
                "2 workflows are running",
                [("count", 2u32.into())],
            ))["params"],
            json!({ "count": 2 })
        );
    }

    #[test]
    fn storage_errors_use_their_variant() {
        assert_eq!(
            payload(&StorageError::InvalidKey.into()),
            json!({
                "code": "storage.invalidKey",
                "message": "the database could not be opened with the provided key"
            })
        );
        assert_eq!(
            payload(
                &StorageError::UnsupportedSchema {
                    found: 9,
                    supported: 7
                }
                .into()
            ),
            json!({
                "code": "storage.unsupportedSchema",
                "message": "database schema version 9 is newer than supported version 7",
                "params": { "found": 9, "supported": 7 }
            })
        );
        assert_eq!(
            payload(&StorageError::CredentialsRequired("system key").into())["code"],
            "storage.credentialsRequired.systemKey"
        );
        assert_eq!(
            payload(&StorageError::from(CryptoError::PasswordTooShort).into()),
            json!({
                "code": "storage.crypto.passwordTooShort",
                "message": "the password must be at least 8 characters long",
                "params": { "count": 8 }
            })
        );
        assert_eq!(
            payload(&StorageError::LayoutInUse(Vec::new()).into())["code"],
            "storage.layoutInUseNone"
        );
    }

    #[test]
    fn agent_errors_use_the_client_and_ipc_codes() {
        assert_eq!(
            payload(&ClientError::NotRunning.into()),
            json!({ "code": "agent.notRunning", "message": "the DeckPilot agent is not running" })
        );
        assert_eq!(payload(&ClientError::Closed.into())["code"], "agent.closed");
        assert_eq!(
            payload(&ClientError::Protocol("bad frame".into()).into()),
            json!({
                "code": "agent.protocol",
                "message": "unexpected message from the agent: bad frame",
                "params": { "detail": "bad frame" }
            })
        );
        let too_old = ClientError::Unsupported {
            request: "listVariables".into(),
            needs: 9,
            protocol: 7,
            agent: "0.8.0".into(),
        };
        assert_eq!(
            payload(&too_old.into())["params"],
            json!({ "agent": "0.8.0", "request": "listVariables", "needs": 9, "protocol": 7 })
        );
        assert_eq!(
            payload(&ClientError::Agent(IpcError::new(ErrorCode::Locked, "locked")).into()),
            json!({ "code": "agent.locked", "message": "locked" })
        );
        let plugin = IpcError::new(ErrorCode::Failed, "the plugin host crashed")
            .with_reason(deckpilot_ipc::plugins::reason::HOST_FAILED);
        assert_eq!(
            payload(&ClientError::Agent(plugin).into()),
            json!({
                "code": "agent.failed",
                "message": "the plugin host crashed",
                "params": { "reason": "hostFailed" }
            })
        );
    }

    #[test]
    fn core_errors_use_their_family() {
        assert_eq!(
            payload(&RegistryError::EmptyName.into()),
            json!({ "code": "registry.emptyName", "message": "action name must not be empty" })
        );
        assert_eq!(
            payload(&EngineError::NoActiveProfile.into())["code"],
            "engine.noActiveProfile"
        );
        assert_eq!(
            payload(&EngineError::Context(ProjectContextError::UnknownProject).into())["code"],
            "projectContext.unknownProject"
        );
        assert_eq!(
            payload(&CatalogError::InUse(3).into()),
            json!({
                "code": "catalog.inUse",
                "message": "the project is still used in 3 place(s); remove those references first",
                "params": { "count": 3 }
            })
        );
        assert_eq!(
            payload(&ProfileIssue::NotFound("layout").into()),
            json!({
                "code": "profile.notFound.layout",
                "message": "layout does not exist in this profile"
            })
        );
        assert_eq!(
            payload(&ProfileIssue::TooManyIcons.into())["params"],
            json!({ "max": deckpilot_core::MAX_ICONS })
        );
    }

    #[test]
    fn camel_case_of_names() {
        assert_eq!(camel("profile"), "profile");
        assert_eq!(camel("resource binding"), "resourceBinding");
        assert_eq!(camel("shared variable"), "sharedVariable");
    }
}
