//! Opens a registry client with the roots this build trusts and refreshes
//! a local registry site, as the agent does (`tests/embedded_roots.rs`
//! builds and runs it):
//!
//! ```text
//! registry_roots <site/v1 directory> <state directory>
//! ```
//!
//! The configured `roots.json` comes from `DECKPILOT_PLUGIN_REGISTRY_ROOTS`
//! (the agent's variable for `--plugin-registry-roots`) and goes through
//! `roots::choose` like the agent's. Prints one line, `roots=<embedded|
//! file|none> ignored=<bool> fetches=<n> result=<...>`; without roots
//! nothing is fetched.

use std::cell::Cell;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use deckpilot_plugin_registry::roots::{self, RootsSource};
use deckpilot_plugin_registry::{
    FetchError, FileTrustStore, Policy, Refresh, RegistryClient, RegistrySource,
};

struct Directory {
    root: PathBuf,
    fetches: Cell<usize>,
}

impl RegistrySource for Directory {
    fn fetch(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
        self.fetches.set(self.fetches.get() + 1);
        if path.split('/').any(|part| part.is_empty() || part == "..") {
            return Err(FetchError::NotFound);
        }
        let file = std::fs::File::open(self.root.join(path)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => FetchError::NotFound,
            _ => FetchError::Unavailable(e.to_string()),
        })?;
        let mut bytes = Vec::new();
        file.take(
            u64::try_from(max_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)
        .map_err(|e| FetchError::Unavailable(e.to_string()))?;
        Ok(bytes)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [site, state] = args.as_slice() else {
        eprintln!("usage: registry_roots <site/v1 directory> <state directory>");
        std::process::exit(2);
    };
    let configured = std::env::var_os("DECKPILOT_PLUGIN_REGISTRY_ROOTS")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let choice = roots::choose(configured.as_deref());
    let source = Directory {
        root: PathBuf::from(site),
        fetches: Cell::new(0),
    };
    let kind = match &choice.source {
        None => "none",
        Some(RootsSource::Embedded(_)) => "embedded",
        Some(RootsSource::File(_)) => "file",
    };
    let result = match &choice.source {
        None => "not-configured".to_owned(),
        Some(roots) => match roots.load().and_then(|role| {
            RegistryClient::open(
                role,
                FileTrustStore::new(Path::new(state)),
                Policy::default(),
            )
            .map_err(|e| e.to_string())
        }) {
            Err(e) => format!("error: {e}"),
            Ok(mut client) => match client.refresh(&source, chrono::Utc::now()) {
                Ok(Refresh::Updated { seq }) => format!("updated seq {seq}"),
                Ok(other) => format!("{other:?}"),
                Err(e) => format!("refused: {e}"),
            },
        },
    };
    println!(
        "roots={kind} ignored={} fetches={} result={result}",
        choice.ignored.is_some(),
        source.fetches.get()
    );
}
