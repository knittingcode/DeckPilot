//! Optional smoke tests against real GitHub, GitLab and Jira services.
//!
//! **Off by default.** Every test is `#[ignore]` and, even when ignored
//! tests are run, does nothing unless `DECKPILOT_SMOKE=1` and the
//! service's own variables are set. Use isolated test accounts with
//! read-only tokens; never personal or production credentials. See
//! “Testing” in `docs/integration-modules.md`.
//!
//! **Read-only.** Only action types whose declared effect is `readOnly`
//! are run (checked before each run), and those send only `GET` requests.
//! Nothing is created, changed or deleted on any service.
//!
//! ```text
//! DECKPILOT_SMOKE=1 DECKPILOT_SMOKE_GITHUB_TOKEN=… \
//!   cargo test -p deckpilot-modules --test live_smoke -- --ignored --nocapture
//! ```

use deckpilot_core::graph::ActionEffect;
use deckpilot_core::{
    ActionParameters, ActionRegistry, ActionResult, ActionTypeId, CredentialId, ExecutionContext,
    MachineConfig, MemoryCredentials, ModuleId, Os, ParameterValue, Profile, Secret,
};
use deckpilot_modules::builtin;

/// The credential name the token is stored under for the run.
const CREDENTIAL: &str = "smoke.token";

fn var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// The service's variables, or `None` (with a note) when the smoke tests
/// are not switched on for it.
fn enabled(service: &str, required: &[&str]) -> Option<Vec<String>> {
    if var("DECKPILOT_SMOKE").as_deref() != Some("1") {
        eprintln!("{service}: skipped (set DECKPILOT_SMOKE=1 to run smoke tests)");
        return None;
    }
    let values: Option<Vec<String>> = required.iter().map(|name| var(name)).collect();
    if values.is_none() {
        eprintln!("{service}: skipped (set {})", required.join(", "));
    }
    values
}

struct Smoke {
    module: &'static str,
    secret: String,
    registry: ActionRegistry,
}

impl Smoke {
    fn new(module: &'static str, secret: String, config: &ActionParameters) -> Self {
        let mut modules = builtin(Os::current());
        let id = ModuleId::new(module).unwrap();
        modules
            .set_config(
                &id,
                &config
                    .clone()
                    .with("credential", ParameterValue::Text(CREDENTIAL.to_owned())),
            )
            .unwrap_or_else(|e| panic!("{module}: the settings are refused: {e}"));
        Self {
            module,
            secret,
            registry: modules.action_registry(),
        }
    }

    /// Runs a read-only action with `secret` stored as the credential.
    fn run_with(&self, action: &str, parameters: &ActionParameters, secret: &str) -> ActionResult {
        let id = ActionTypeId::new(action).unwrap();
        let definition = self
            .registry
            .definition(&id)
            .unwrap_or_else(|| panic!("{action} is not available"));
        assert_eq!(
            definition.node.effect,
            ActionEffect::ReadOnly,
            "{action} is not read-only; smoke tests run only read-only actions"
        );
        let parameters = self.registry.normalize(&id, parameters).unwrap();
        let profile = Profile::new("Smoke");
        let machine = MachineConfig::new(Os::current());
        let credentials = MemoryCredentials::new()
            .with(CredentialId::new(CREDENTIAL).unwrap(), Secret::new(secret));
        let context =
            ExecutionContext::new(&profile, None, &machine).with_credentials(&credentials);
        let result = self.registry.dispatch(&id, &parameters, &context);
        self.assert_no_secret(&result);
        result
    }

    fn run(&self, action: &str, parameters: &ActionParameters) -> ActionResult {
        let result = self.run_with(action, parameters, &self.secret);
        // Only counts and codes are printed; outputs may be private.
        eprintln!(
            "{}: {action}: {:?} {}",
            self.module,
            result.status,
            result.code.as_deref().unwrap_or("")
        );
        result
    }

    fn assert_no_secret(&self, result: &ActionResult) {
        let shown = format!("{result:?}");
        let mut forms = vec![self.secret.clone()];
        if let Some((_, token)) = self.secret.split_once(':') {
            forms.push(token.to_owned());
        }
        for form in forms.iter().filter(|f| f.len() >= 8) {
            assert!(
                !shown.contains(form.as_str()),
                "{}: a result contains the secret",
                self.module
            );
        }
    }

    fn expect_success(&self, action: &str, parameters: &ActionParameters) -> ActionResult {
        let result = self.run(action, parameters);
        assert!(
            result.is_success(),
            "{}: {action} failed with {:?}",
            self.module,
            result.code
        );
        result
    }

    /// A made-up token is refused as unauthorized, not as anything else.
    fn expect_unauthorized(&self, action: &str, wrong: &str) {
        let result = self.run_with(action, &ActionParameters::new(), wrong);
        assert_eq!(
            result.code.as_deref(),
            Some("unauthorized"),
            "{}: a wrong token gave {:?}",
            self.module,
            result.code
        );
    }
}

fn text(value: &str) -> ParameterValue {
    ParameterValue::Text(value.to_owned())
}

fn base_config(url_key: &str, url: Option<String>) -> ActionParameters {
    let mut config = ActionParameters::new();
    if let Some(url) = url {
        config = config.with(url_key, text(&url));
    }
    if var("DECKPILOT_SMOKE_ALLOW_PRIVATE").as_deref() == Some("1") {
        config = config.with("allowPrivateHost", ParameterValue::Boolean(true));
    }
    config
}

/// `DECKPILOT_SMOKE_GITHUB_TOKEN` (required), `DECKPILOT_SMOKE_GITHUB_API_URL`
/// (default `https://api.github.com`), `DECKPILOT_SMOKE_GITHUB_REPOSITORY`
/// (`owner/name`, optional).
#[test]
#[ignore = "real service; needs DECKPILOT_SMOKE=1 and DECKPILOT_SMOKE_GITHUB_TOKEN"]
fn github_read_only_smoke() {
    let Some(values) = enabled("github", &["DECKPILOT_SMOKE_GITHUB_TOKEN"]) else {
        return;
    };
    let smoke = Smoke::new(
        "github",
        values[0].clone(),
        &base_config("apiUrl", var("DECKPILOT_SMOKE_GITHUB_API_URL")),
    );
    let user = smoke.expect_success("github.current-user", &ActionParameters::new());
    assert_ne!(user.data["login"], "");
    smoke.expect_success(
        "github.pull-requests",
        &ActionParameters::new().with("filter", ParameterValue::Choice("author-me".into())),
    );
    smoke.expect_success("github.review-requests", &ActionParameters::new());
    if let Some(repository) = var("DECKPILOT_SMOKE_GITHUB_REPOSITORY") {
        let repo = ActionParameters::new().with("repository", text(&repository));
        let info = smoke.expect_success("github.repository", &repo);
        smoke.expect_success("github.pull-requests", &repo);
        let branch = info.data["defaultBranch"].clone();
        let checks = smoke.expect_success("github.checks", &repo.with("ref", text(&branch)));
        assert!(["ok", "failing", "pending", "none"].contains(&checks.data["state"].as_str()));
    }
    smoke.expect_unauthorized(
        "github.current-user",
        "ghp_deckpilotSmokeNotAToken0000000000",
    );
}

/// `DECKPILOT_SMOKE_GITLAB_TOKEN` (required), `DECKPILOT_SMOKE_GITLAB_API_URL`
/// (default `https://gitlab.com/api/v4`), `DECKPILOT_SMOKE_GITLAB_PROJECT`
/// (`group/project`, optional).
#[test]
#[ignore = "real service; needs DECKPILOT_SMOKE=1 and DECKPILOT_SMOKE_GITLAB_TOKEN"]
fn gitlab_read_only_smoke() {
    let Some(values) = enabled("gitlab", &["DECKPILOT_SMOKE_GITLAB_TOKEN"]) else {
        return;
    };
    let smoke = Smoke::new(
        "gitlab",
        values[0].clone(),
        &base_config("apiUrl", var("DECKPILOT_SMOKE_GITLAB_API_URL")),
    );
    let user = smoke.expect_success("gitlab.current-user", &ActionParameters::new());
    assert_ne!(user.data["username"], "");
    smoke.expect_success(
        "gitlab.merge-requests",
        &ActionParameters::new().with("filter", ParameterValue::Choice("author-me".into())),
    );
    smoke.expect_success("gitlab.review-requests", &ActionParameters::new());
    if let Some(project) = var("DECKPILOT_SMOKE_GITLAB_PROJECT") {
        let parameters = ActionParameters::new().with("project", text(&project));
        let info = smoke.expect_success("gitlab.project", &parameters);
        smoke.expect_success("gitlab.merge-requests", &parameters);
        if let Some(branch) = info.data.get("defaultBranch").cloned() {
            smoke.expect_success("gitlab.pipeline", &parameters.with("ref", text(&branch)));
        }
    }
    smoke.expect_unauthorized("gitlab.current-user", "glpat-deckpilotSmokeNotAToken00");
}

/// `DECKPILOT_SMOKE_JIRA_SITE` and `DECKPILOT_SMOKE_JIRA_CREDENTIAL`
/// (required; `email:api-token` for Cloud, a personal access token for
/// Data Center), `DECKPILOT_SMOKE_JIRA_DEPLOYMENT` (`cloud` or
/// `data-center`, default `cloud`), `DECKPILOT_SMOKE_JIRA_ISSUE` (a key,
/// optional), `DECKPILOT_SMOKE_JIRA_FILTER` (a saved filter's number,
/// optional).
#[test]
#[ignore = "real service; needs DECKPILOT_SMOKE=1, DECKPILOT_SMOKE_JIRA_SITE and DECKPILOT_SMOKE_JIRA_CREDENTIAL"]
fn jira_read_only_smoke() {
    let Some(values) = enabled(
        "jira",
        &[
            "DECKPILOT_SMOKE_JIRA_SITE",
            "DECKPILOT_SMOKE_JIRA_CREDENTIAL",
        ],
    ) else {
        return;
    };
    let deployment = var("DECKPILOT_SMOKE_JIRA_DEPLOYMENT").unwrap_or_else(|| "cloud".into());
    let config = base_config("siteUrl", Some(values[0].clone()))
        .with("deployment", ParameterValue::Choice(deployment.clone()));
    let smoke = Smoke::new("jira", values[1].clone(), &config);
    let me = smoke.expect_success("jira.myself", &ActionParameters::new());
    assert_ne!(me.data["account"], "");
    smoke.expect_success("jira.issues", &ActionParameters::new());
    if let Some(issue) = var("DECKPILOT_SMOKE_JIRA_ISSUE") {
        smoke.expect_success(
            "jira.issue",
            &ActionParameters::new().with("issue", text(&issue)),
        );
    }
    if let Some(filter) = var("DECKPILOT_SMOKE_JIRA_FILTER") {
        let id: i64 = filter
            .parse()
            .expect("DECKPILOT_SMOKE_JIRA_FILTER is a number");
        smoke.expect_success(
            "jira.issues",
            &ActionParameters::new()
                .with("source", ParameterValue::Choice("filter".into()))
                .with("filterId", ParameterValue::Integer(id)),
        );
    }
    let wrong = if deployment == "cloud" {
        "smoke@example.invalid:deckpilotSmokeNotAToken"
    } else {
        "deckpilotSmokeNotAToken000000"
    };
    smoke.expect_unauthorized("jira.myself", wrong);
}
