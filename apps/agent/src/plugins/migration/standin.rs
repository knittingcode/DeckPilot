//! TEST HOOK, never in a release build (features `plugins` and
//! `test-hooks`): the requests of a built-in module that talks to a server
//! (`github`, `gitlab`, `jira`) go to a stand-in on this computer, so an
//! agent test can run the built-in from a key or a workflow node without
//! reaching the real service (`tests/plugins_cutover.rs`,
//! `tests/plugins_cutover_servers.rs`).
//!
//! With a module's variable ([`STAND_INS`]) set to
//! `http://127.0.0.1:<port>[/path]`, the module's handlers are built with
//! that address and private hosts allowed, whatever the stored settings
//! say. The stored settings themselves are untouched: the module registry,
//! `deckpilot.db` and therefore the migration's eligibility and what it
//! carries (the server the user entered) see what the user saved. It is
//! the built-in's counterpart of `DECKPILOT_TEST_PLUGIN_HTTP`, the canned
//! answers of plugin requests.

use deckpilot_core::{
    ActionHandler, ActionParameters, FeatureModule, ModuleId, ModuleManifest, ModuleRegistry,
    ParameterValue,
};
use deckpilot_modules::{github, gitlab, jira};

/// One module that can be pointed at a stand-in.
struct StandIn {
    /// The variable with the stand-in's address.
    env: &'static str,
    module: fn() -> ModuleId,
    build: fn() -> Box<dyn FeatureModule>,
    /// The module's setting with the server's address.
    address: &'static str,
    /// Its switch for private hosts.
    private: &'static str,
}

/// The modules and their variables: `DECKPILOT_TEST_BUILTIN_GITHUB_API`
/// (`http://127.0.0.1:<port>`), `DECKPILOT_TEST_BUILTIN_GITLAB_API`
/// (`http://127.0.0.1:<port>[/path]/api/v4`) and
/// `DECKPILOT_TEST_BUILTIN_JIRA_SITE` (`http://127.0.0.1:<port>[/path]`).
const STAND_INS: &[StandIn] = &[
    StandIn {
        env: "DECKPILOT_TEST_BUILTIN_GITHUB_API",
        module: github::module_id,
        build: || Box::new(github::GithubModule::new()),
        address: github::API_URL,
        private: github::ALLOW_PRIVATE,
    },
    StandIn {
        env: "DECKPILOT_TEST_BUILTIN_GITLAB_API",
        module: gitlab::module_id,
        build: || Box::new(gitlab::GitlabModule::new()),
        address: gitlab::API_URL,
        private: gitlab::ALLOW_PRIVATE,
    },
    StandIn {
        env: "DECKPILOT_TEST_BUILTIN_JIRA_SITE",
        module: jira::module_id,
        build: || Box::new(jira::JiraModule::new()),
        address: jira::SITE_URL,
        private: jira::ALLOW_PRIVATE,
    },
];

/// The stand-in's address in `env`, when an agent test asked for it. Only
/// a loopback `http` URL is taken.
fn address(env: &str) -> Option<String> {
    std::env::var(env)
        .ok()
        .filter(|url| url.starts_with("http://127.0.0.1:"))
}

/// Replaces every module an agent test set a stand-in for; whether any
/// was.
pub fn apply(registry: &mut ModuleRegistry) -> bool {
    let mut any = false;
    for stand_in in STAND_INS {
        let Some(url) = address(stand_in.env) else {
            continue;
        };
        registry.unregister(&(stand_in.module)());
        registry
            .register(Box::new(Pointed {
                inner: (stand_in.build)(),
                address: stand_in.address,
                private: stand_in.private,
                url,
            }))
            .expect("the stand-in replaces the module");
        any = true;
    }
    any
}

/// A built-in module, its handlers pointed at the stand-in.
struct Pointed {
    inner: Box<dyn FeatureModule>,
    address: &'static str,
    private: &'static str,
    url: String,
}

impl std::fmt::Debug for Pointed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pointed")
            .field("module", &self.inner.manifest().id)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl FeatureModule for Pointed {
    fn manifest(&self) -> &ModuleManifest {
        self.inner.manifest()
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        let config = config
            .clone()
            .with(self.address, ParameterValue::Text(self.url.clone()))
            .with(self.private, ParameterValue::Boolean(true));
        self.inner.action_handlers(&config)
    }
}
