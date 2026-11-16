//! TEST HOOK, never in a release build (features `plugins` and
//! `test-hooks`): the built-in `github` module's requests go to a stand-in
//! GitHub on this computer, so an agent test can run the built-in from a
//! key or a workflow node without reaching the real GitHub
//! (`tests/plugins_cutover.rs`).
//!
//! With [`ENV`] set to `http://127.0.0.1:<port>`, the module's handlers
//! are built with that API URL and private hosts allowed, whatever the
//! stored settings say. The stored settings themselves are untouched: the
//! module registry, `deckpilot.db` and therefore the migration's
//! eligibility (`apiUrl` is GitHub's or not) see what the user saved. It
//! is the built-in's counterpart of `DECKPILOT_TEST_PLUGIN_HTTP`, the
//! canned answers of plugin requests.

use deckpilot_core::{
    ActionHandler, ActionParameters, FeatureModule, ModuleManifest, ParameterValue,
};
use deckpilot_modules::github;

/// The stand-in's base URL, `http://127.0.0.1:<port>`.
pub const ENV: &str = "DECKPILOT_TEST_BUILTIN_GITHUB_API";

/// The stand-in's URL, when an agent test asked for it. Only a loopback
/// `http` URL is taken.
pub fn api() -> Option<String> {
    std::env::var(ENV)
        .ok()
        .filter(|url| url.starts_with("http://127.0.0.1:"))
}

/// The built-in module, its handlers pointed at the stand-in.
#[derive(Debug)]
pub struct StandInGithub {
    inner: github::GithubModule,
    api: String,
}

impl StandInGithub {
    pub fn new(api: String) -> Self {
        Self {
            inner: github::GithubModule::new(),
            api,
        }
    }
}

impl FeatureModule for StandInGithub {
    fn manifest(&self) -> &ModuleManifest {
        self.inner.manifest()
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        let config = config
            .clone()
            .with(github::API_URL, ParameterValue::Text(self.api.clone()))
            .with(github::ALLOW_PRIVATE, ParameterValue::Boolean(true));
        self.inner.action_handlers(&config)
    }
}
