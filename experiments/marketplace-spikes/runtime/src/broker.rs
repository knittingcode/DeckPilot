//! Deny-by-default permission broker in the supervisor (design §5.1).
//!
//! `authorize` is the only decision point. The plugin identity is passed in
//! by the supervisor from the channel, never taken from the request.
//!
//! `fs.read` is here because the spike brief asks for it; the design does
//! not make the file system grantable in API 1 (§5.2).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};
use url::Url;

/// Largest file `fs.read` returns.
pub const MAX_READ: u64 = 128 * 1024;

#[derive(Debug, Clone)]
pub enum Grant {
    /// `http.fetch` to exactly these origins (scheme, host, port).
    HttpFetch { origins: Vec<url::Origin> },
    /// `fs.read` of regular files below this directory (stored canonical).
    FsRead { root: PathBuf },
}

impl Grant {
    pub fn http(origins: &[&str]) -> Self {
        Grant::HttpFetch {
            origins: origins
                .iter()
                .map(|o| Url::parse(o).expect("valid origin").origin())
                .collect(),
        }
    }

    pub fn fs_read(root: &Path) -> std::io::Result<Self> {
        Ok(Grant::FsRead {
            root: root.canonicalize()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub plugin: String,
    pub op: String,
    /// A host or a relative path; never a body or a value.
    pub target: String,
    pub allowed: bool,
    pub reason: String,
}

#[derive(Default)]
pub struct Broker {
    grants: HashMap<String, Vec<Grant>>,
    audit: Mutex<Vec<AuditEntry>>,
}

fn deny(msg: impl Into<String>) -> Decision {
    Decision::Deny(msg.into())
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn grant(&mut self, plugin: &str, grant: Grant) {
        self.grants
            .entry(plugin.to_owned())
            .or_default()
            .push(grant);
    }

    pub fn revoke_all(&mut self, plugin: &str) {
        self.grants.remove(plugin);
    }

    pub fn audit(&self) -> Vec<AuditEntry> {
        self.audit.lock().unwrap().clone()
    }

    /// Decides one request. Unknown plugins, unknown ops and malformed
    /// arguments are denied.
    pub fn authorize(&self, plugin: &str, op: &str, args: &Value) -> Decision {
        let decision = self.decide(plugin, op, args);
        let target = match op {
            "http.fetch" => args
                .get("url")
                .and_then(Value::as_str)
                .and_then(|u| Url::parse(u).ok())
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_default(),
            "fs.read" => args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            _ => String::new(),
        };
        let (allowed, reason) = match &decision {
            Decision::Allow => (true, String::new()),
            Decision::Deny(r) => (false, r.clone()),
        };
        self.audit.lock().unwrap().push(AuditEntry {
            plugin: plugin.to_owned(),
            op: op.to_owned(),
            target,
            allowed,
            reason,
        });
        decision
    }

    fn decide(&self, plugin: &str, op: &str, args: &Value) -> Decision {
        let Some(grants) = self.grants.get(plugin) else {
            return deny("no grants for plugin");
        };
        match op {
            "http.fetch" => {
                let Some(raw) = args.get("url").and_then(Value::as_str) else {
                    return deny("url missing");
                };
                let url = match Url::parse(raw) {
                    Ok(u) => u,
                    Err(e) => return deny(format!("bad url: {e}")),
                };
                if url.scheme() != "https" {
                    return deny("only https");
                }
                if !url.username().is_empty() || url.password().is_some() {
                    return deny("userinfo not allowed");
                }
                let origin = url.origin();
                let ok = grants.iter().any(
                    |g| matches!(g, Grant::HttpFetch { origins } if origins.contains(&origin)),
                );
                if ok {
                    Decision::Allow
                } else {
                    deny("origin not granted")
                }
            }
            "fs.read" => {
                let roots: Vec<&PathBuf> = grants
                    .iter()
                    .filter_map(|g| match g {
                        Grant::FsRead { root } => Some(root),
                        _ => None,
                    })
                    .collect();
                if roots.is_empty() {
                    return deny("fs.read not granted");
                }
                let Some(rel) = args.get("path").and_then(Value::as_str) else {
                    return deny("path missing");
                };
                match resolve_below(&roots, rel) {
                    Ok(_) => Decision::Allow,
                    Err(r) => Decision::Deny(r),
                }
            }
            _ => deny(format!("unknown op {op}")),
        }
    }

    /// Authorizes and, if allowed, performs the request. This is what the
    /// supervisor calls for every `call` frame.
    pub fn handle(&self, plugin: &str, op: &str, args: &Value) -> Result<Value, String> {
        if let Decision::Deny(reason) = self.authorize(plugin, op, args) {
            return Err(format!("not-permitted: {reason}"));
        }
        match op {
            "fs.read" => {
                let roots: Vec<&PathBuf> = self.grants[plugin]
                    .iter()
                    .filter_map(|g| match g {
                        Grant::FsRead { root } => Some(root),
                        _ => None,
                    })
                    .collect();
                // Resolved again right before the open; a production broker
                // must open relative to a directory handle instead (see
                // RESULTS.md: cap-std / openat2 RESOLVE_BENEATH).
                let path = resolve_below(&roots, args["path"].as_str().unwrap())?;
                let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
                if meta.len() > MAX_READ {
                    return Err("file too large".into());
                }
                let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                Ok(json!({ "text": text }))
            }
            // No network in the spike: the decision is what is under test.
            "http.fetch" => Ok(json!({ "notExecuted": true, "url": args["url"] })),
            _ => Err("unreachable".into()),
        }
    }
}

/// Resolves `rel` against each root and returns the canonical path if it is
/// a regular file that stays below that root after resolving symlinks.
fn resolve_below(roots: &[&PathBuf], rel: &str) -> Result<PathBuf, String> {
    if rel.is_empty() || rel.contains('\0') {
        return Err("bad path".into());
    }
    let p = Path::new(rel);
    for c in p.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => return Err("parent components not allowed".into()),
            Component::RootDir | Component::Prefix(_) => {
                return Err("absolute paths not allowed".into());
            }
        }
    }
    for root in roots {
        let Ok(canon) = root.join(p).canonicalize() else {
            continue;
        };
        if !canon.starts_with(root) {
            return Err("path escapes granted directory".into());
        }
        if !canon.is_file() {
            return Err("not a regular file".into());
        }
        return Ok(canon);
    }
    Err("not found below a granted directory".into())
}
