//! The `deckpilot-registry` command line.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};
use deckpilot_plugin_manifest::PluginId;
use semver::Version;

use super::catalog::{self, NewVersion, PluginInfo};
use super::index::{self, BuildIndex, DEFAULT_KEEP, DEFAULT_TIMESTAMP_VALIDITY_DAYS};
use super::keys::{self, KeyRole, Password, SigningKey, generate_key};
use super::keyset::{self, Rotation};
use super::layout::{Workspace, read_json};
use super::verify;
use super::{PublishError, Result, fail};
use crate::model::{
    IndexPublisher, Revocation, RevocationAction, RevokedVersions, Role, VersionSource,
};

/// Environment variable read for a key password when no option names one.
pub const DEFAULT_PASSWORD_ENV: &str = "DECKPILOT_REGISTRY_KEY_PASSWORD";

pub const USAGE: &str = "\
deckpilot-registry: offline publishing tooling for the static DeckPilot plugin registry

Every command works on files in --dir (default: the current directory):
roots.json, registry/ (source data), releases/ (release assets) and
site/v1/ (the Pages tree). Times default to the system clock (--now RFC 3339
overrides). Passwords: --password-stdin, --password-env NAME, or
DECKPILOT_REGISTRY_KEY_PASSWORD; never printed.

  keys generate --role root|index|timestamp|publisher --out FILE [--no-password]
      Local and test keys only; production keys come from the key ceremony.
  keyset init --root PUB [--root PUB] [--root-threshold N] --index PUB --timestamp PUB
              [--seq-floor N] [--first-party]
  keyset rotate [--root PUB]... [--root-threshold N] [--index PUB] [--timestamp PUB]
                [--seq-floor N] [--seq-floor-reset] [--first-party | --no-first-party]
      --first-party authorizes the reserved namespace deckpilot (first_party in
      the root-signed key set); --no-first-party withdraws it.
  keyset sign --key FILE [--version N]
  package --plugin-dir DIR --module FILE.wasm --out FILE.dpplugin [--first-party]
      Builds a package: manifest.json, LICENSE and the files the manifest names
      from DIR, the module as plugin.wasm (plugin ABI 1, checked). A deckpilot.*
      plugin needs --first-party.
  publisher --namespace NS --github-id N --login LOGIN [--display-name NAME] [--verified]
            [--first-party]                           (--first-party: namespace deckpilot)
  add-version --package FILE.dpplugin [--plugin-detail plugin.json]
              --source-repository OWNER/NAME --tag TAG --commit SHA [--source-path PATH]
              --builder VERSION [--attestation URL] [--first-party]
      A deckpilot.* package needs --first-party, a key set authorizing deckpilot
      and a verified deckpilot publisher.
  yank --id ID --version V --reason TEXT [--advisory URL]
  revoke --id ID --versions V[,V...]|* --action disable|yank --reason TEXT [--advisory URL]
  lift-revocation --id ID [--versions V[,V...]|*]    (only after --seq-floor-reset)
  build-index --source-commit SHA [--min-app-version V] [--keep N]
  sign-index --key FILE [--seq N]
  timestamp --key FILE [--expires-in 7d]             (at most 14d)
  verify-site [--roots FILE] [--state DIR] [--min-remaining 3d] [--require-packages]

First-party release artifacts (the publisher signature, crate::publisher):
  release sign --package FILE.dpplugin --id ID --version V (--key FILE | --key-env NAME)
               --out FILE.minisig
      Signs a deckpilot.* package as ID V (from the release tag; the package must
      say the same) with the publisher key, after the verifier's own checks, which
      run before the key is loaded. --key-env reads the secret key text from an
      environment variable.
  release verify --package FILE.dpplugin --signature FILE.minisig... --id ID --version V
                 --sha256 HEX --size N --app-version V --plugin-api MAJOR.MINOR
                 [--platform macos|windows|linux] [--keys FILE] [--released-pins FILE]
                 [--revocations FILE] [--audit FILE.jsonl] [--unpack DIR]
      Verifies an artifact against the claim (id, version, sha256, size), the pinned
      publisher keys (--keys: others, for tests), revocations, the host and the
      released pins (rollback); appends the outcome to --audit; --unpack writes the
      files of a verified artifact. Authenticity only: it grants no permission.
";

/// Where a command reads secrets and writes its output. Tests pass their
/// own; the binary passes the process environment, stdin and stdout.
pub struct Io<'a> {
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub stdin: &'a mut dyn BufRead,
    pub out: &'a mut dyn Write,
}

impl fmt::Debug for Io<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Io").finish_non_exhaustive()
    }
}

/// Parsed `--name value` and `--flag` options.
struct Opts {
    values: Vec<(String, Option<String>)>,
    used: RefCell<BTreeSet<String>>,
}

impl Opts {
    fn parse(args: &[String], flags: &[&str]) -> Result<Self> {
        let mut values = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let Some(name) = arg.strip_prefix("--") else {
                return fail(format!("unexpected argument `{arg}`"));
            };
            if let Some((name, value)) = name.split_once('=') {
                values.push((name.to_owned(), Some(value.to_owned())));
            } else if flags.contains(&name) {
                values.push((name.to_owned(), None));
            } else {
                let value = iter
                    .next()
                    .ok_or_else(|| PublishError(format!("--{name} needs a value")))?;
                values.push((name.to_owned(), Some(value.clone())));
            }
        }
        Ok(Self {
            values,
            used: RefCell::new(BTreeSet::new()),
        })
    }

    fn all(&self, name: &str) -> Vec<String> {
        self.used.borrow_mut().insert(name.to_owned());
        self.values
            .iter()
            .filter(|(n, _)| n == name)
            .filter_map(|(_, v)| v.clone())
            .collect()
    }

    fn value(&self, name: &str) -> Result<Option<String>> {
        let mut all = self.all(name);
        if all.len() > 1 {
            return fail(format!("--{name} is given more than once"));
        }
        Ok(all.pop())
    }

    fn required(&self, name: &str) -> Result<String> {
        self.value(name)?
            .ok_or_else(|| PublishError(format!("--{name} is required")))
    }

    fn flag(&self, name: &str) -> bool {
        self.used.borrow_mut().insert(name.to_owned());
        self.values.iter().any(|(n, v)| n == name && v.is_none())
    }

    fn path(&self, name: &str) -> Result<Option<PathBuf>> {
        Ok(self.value(name)?.map(PathBuf::from))
    }

    fn parsed<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>>
    where
        T::Err: fmt::Display,
    {
        self.value(name)?
            .map(|text| {
                text.parse()
                    .map_err(|e| PublishError(format!("--{name}: {e}")))
            })
            .transpose()
    }

    fn workspace(&self) -> Result<Workspace> {
        Ok(Workspace::new(
            self.path("dir")?.unwrap_or_else(|| PathBuf::from(".")),
        ))
    }

    fn now(&self) -> Result<DateTime<Utc>> {
        match self.value("now")? {
            Some(text) => DateTime::parse_from_rfc3339(&text)
                .map(|t| t.with_timezone(&Utc))
                .map_err(|e| PublishError(format!("--now: {e}"))),
            None => Ok(Utc::now()),
        }
    }

    fn duration(&self, name: &str, default: TimeDelta) -> Result<TimeDelta> {
        self.value(name)?
            .map_or(Ok(default), |text| parse_duration(&text))
            .map_err(|e| PublishError(format!("--{name}: {e}")))
    }

    /// Fails on any option the command did not read.
    fn finish(&self) -> Result<()> {
        let used = self.used.borrow();
        if let Some((name, _)) = self.values.iter().find(|(n, _)| !used.contains(n)) {
            return fail(format!("unknown option --{name}"));
        }
        Ok(())
    }
}

/// `30m`, `12h`, `7d` or `3600s`.
pub(crate) fn parse_duration(text: &str) -> Result<TimeDelta> {
    let bad = || PublishError(format!("`{text}` is not a duration like 7d, 12h or 30m"));
    let split = text.len().checked_sub(1).ok_or_else(bad)?;
    let (number, unit) = text.split_at(split);
    let number: i64 = number.parse().map_err(|_| bad())?;
    let delta = match unit {
        "s" => TimeDelta::try_seconds(number),
        "m" => TimeDelta::try_minutes(number),
        "h" => TimeDelta::try_hours(number),
        "d" => TimeDelta::try_days(number),
        _ => None,
    };
    delta.filter(|d| *d >= TimeDelta::zero()).ok_or_else(bad)
}

fn password(opts: &Opts, io: &mut Io<'_>) -> Result<Option<Password>> {
    let from_stdin = opts.flag("password-stdin");
    let env_name = opts.value("password-env")?;
    if from_stdin && env_name.is_some() {
        return fail("give either --password-stdin or --password-env");
    }
    if from_stdin {
        let mut line = zeroize::Zeroizing::new(String::new());
        io.stdin
            .read_line(&mut line)
            .map_err(|e| PublishError(format!("stdin: {e}")))?;
        let text = line.trim_end_matches(['\n', '\r']);
        if text.is_empty() {
            return fail("the password from stdin is empty");
        }
        return Ok(Some(Password::new(text.to_owned())));
    }
    let name = env_name.as_deref().unwrap_or(DEFAULT_PASSWORD_ENV);
    match (io.env)(name) {
        Some(value) if !value.is_empty() => Ok(Some(Password::new(value))),
        _ if env_name.is_some() => fail(format!("the environment variable {name} is not set")),
        _ => Ok(None),
    }
}

fn signing_key(opts: &Opts, io: &mut Io<'_>) -> Result<SigningKey> {
    let path = opts
        .path("key")?
        .ok_or_else(|| PublishError("--key is required".into()))?;
    let password = password(opts, io)?;
    SigningKey::load(&path, password.as_ref())
}

/// `--key FILE` or `--key-env NAME` (the secret key text in an environment
/// variable, for a CI secret that never touches the disk).
fn release_key(opts: &Opts, io: &mut Io<'_>) -> Result<SigningKey> {
    let file = opts.path("key")?;
    let env = opts.value("key-env")?;
    let password = password(opts, io)?;
    match (file, env) {
        (Some(path), None) => SigningKey::load(&path, password.as_ref()),
        (None, Some(name)) => {
            let text =
                zeroize::Zeroizing::new((io.env)(&name).filter(|v| !v.is_empty()).ok_or_else(
                    || PublishError(format!("the environment variable {name} is not set")),
                )?);
            SigningKey::from_text(&text, &format!("${name}"), password.as_ref())
        }
        _ => fail("give either --key FILE or --key-env NAME"),
    }
}

fn role_of(opts: &Opts, name: &str) -> Result<Option<Role>> {
    opts.path(name)?
        .map(|path| {
            Ok(Role {
                keys: vec![keys::read_public(&path)?],
                threshold: 1,
            })
        })
        .transpose()
}

fn roots_of(opts: &Opts) -> Result<Option<Vec<String>>> {
    let files = opts.all("root");
    if files.is_empty() {
        return Ok(None);
    }
    files
        .iter()
        .map(|f| keys::read_public(Path::new(f)))
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn plugin_id(text: &str) -> Result<PluginId> {
    PluginId::parse(text).map_err(|e| PublishError(format!("--id: {e}")))
}

fn versions_of(text: &str) -> Result<RevokedVersions> {
    if text == "*" {
        return Ok(RevokedVersions::All);
    }
    text.split(',')
        .map(|v| {
            Version::parse(v.trim()).map_err(|e| PublishError(format!("--versions `{v}`: {e}")))
        })
        .collect::<Result<Vec<_>>>()
        .map(RevokedVersions::List)
}

fn action_of(text: &str) -> Result<RevocationAction> {
    match text {
        "disable" => Ok(RevocationAction::Disable),
        "yank" => Ok(RevocationAction::Yank),
        other => fail(format!("--action `{other}`: disable or yank")),
    }
}

/// Runs one command. `args` excludes the program name.
#[allow(clippy::too_many_lines)] // one arm per command
pub fn run(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let (command, rest) = match args {
        [group, sub, rest @ ..] if group == "keys" || group == "keyset" || group == "release" => {
            (format!("{group} {sub}"), rest)
        }
        [command, rest @ ..] => (command.clone(), rest),
        [] => ("help".to_owned(), args),
    };
    let flags = [
        "no-password",
        "password-stdin",
        "seq-floor-reset",
        "verified",
        "require-packages",
        "first-party",
        "no-first-party",
    ];
    let opts = Opts::parse(rest, &flags)?;
    let ws = opts.workspace()?;
    let message = match command.as_str() {
        "help" | "--help" | "-h" => USAGE.to_owned(),
        "package" => {
            let dir = PathBuf::from(opts.required("plugin-dir")?);
            let module = PathBuf::from(opts.required("module")?);
            let out = PathBuf::from(opts.required("out")?);
            let first_party = opts.flag("first-party");
            opts.finish()?;
            let built = super::pack::package(&dir, &module, first_party)?;
            std::fs::write(&out, &built.bytes)
                .map_err(|e| PublishError(format!("{}: {e}", out.display())))?;
            format!(
                "{} {} packaged to {} (sha256 {}, {} bytes)",
                built.manifest.id,
                built.manifest.version,
                out.display(),
                built.integrity.package.sha256,
                built.integrity.package.size
            )
        }
        "keys generate" => {
            let role = KeyRole::parse(&opts.required("role")?)?;
            let out = PathBuf::from(opts.required("out")?);
            // --no-password ignores DECKPILOT_REGISTRY_KEY_PASSWORD but
            // conflicts with an explicit password option.
            let no_password = opts.flag("no-password");
            let explicit = opts.flag("password-stdin") || opts.value("password-env")?.is_some();
            let password = if no_password {
                None
            } else {
                password(&opts, io)?
            };
            opts.finish()?;
            let password = match (no_password, password) {
                (true, _) if explicit => {
                    return fail("--no-password conflicts with a password option");
                }
                (true, _) => None,
                (false, Some(password)) => Some(password),
                (false, None) => {
                    return fail(
                        "a password is required (--password-stdin, --password-env or \
                         DECKPILOT_REGISTRY_KEY_PASSWORD); --no-password for throwaway keys",
                    );
                }
            };
            generate_key(role, &out, password.as_ref())?;
            format!(
                "{} key written to {} (public key: {})",
                role.as_str(),
                out.display(),
                keys::public_path(&out).display()
            )
        }
        "keyset init" => {
            let root = Role {
                keys: roots_of(&opts)?.ok_or_else(|| PublishError("--root is required".into()))?,
                threshold: opts.parsed("root-threshold")?.unwrap_or(1),
            };
            let index = role_of(&opts, "index")?
                .ok_or_else(|| PublishError("--index is required".into()))?;
            let timestamp = role_of(&opts, "timestamp")?
                .ok_or_else(|| PublishError("--timestamp is required".into()))?;
            let seq_floor = opts.parsed("seq-floor")?.unwrap_or(0);
            let first_party = opts.flag("first-party");
            opts.finish()?;
            keyset::init(&ws, &root, index, timestamp, seq_floor, first_party)?;
            format!(
                "wrote roots.json and site/v1/keys/1.json{}; sign it with `keyset sign`",
                if first_party {
                    " (authorizes the first-party namespace deckpilot)"
                } else {
                    ""
                }
            )
        }
        "keyset rotate" => {
            let rotation = Rotation {
                root: roots_of(&opts)?,
                root_threshold: opts.parsed("root-threshold")?,
                index: role_of(&opts, "index")?,
                timestamp: role_of(&opts, "timestamp")?,
                seq_floor: opts.parsed("seq-floor")?,
                seq_floor_reset: opts.flag("seq-floor-reset"),
                first_party: match (opts.flag("first-party"), opts.flag("no-first-party")) {
                    (true, true) => return fail("give either --first-party or --no-first-party"),
                    (true, false) => Some(true),
                    (false, true) => Some(false),
                    (false, false) => None,
                },
            };
            opts.finish()?;
            let keyset = keyset::rotate(&ws, rotation)?;
            format!(
                "wrote site/v1/keys/{}.json (seq_floor {}{}{}); sign it with the current roots \
                 and, if they change, the new roots",
                keyset.version,
                keyset.seq_floor,
                if keyset.seq_floor_reset {
                    ", reset"
                } else {
                    ""
                },
                if keyset.authorizes_first_party() {
                    ", first-party namespace deckpilot authorized"
                } else {
                    ""
                }
            )
        }
        "keyset sign" => {
            let version = opts.parsed("version")?;
            let key = signing_key(&opts, io)?;
            opts.finish()?;
            keyset::sign(&ws, version, &key)?
        }
        "publisher" => {
            let entry = IndexPublisher {
                namespace: opts.required("namespace")?,
                github_id: opts
                    .parsed("github-id")?
                    .ok_or_else(|| PublishError("--github-id is required".into()))?,
                login: opts.required("login")?,
                display_name: opts.value("display-name")?,
                verified: opts.flag("verified"),
            };
            let first_party = opts.flag("first-party");
            opts.finish()?;
            let message = format!("publisher {} ({})", entry.namespace, entry.login);
            catalog::publisher(&ws, &entry, first_party)?;
            message
        }
        "add-version" => {
            let package = catalog::read_package(Path::new(&opts.required("package")?))?;
            let info: Option<PluginInfo> = opts
                .path("plugin-detail")?
                .map(|path| read_json(&path))
                .transpose()?;
            let new = NewVersion {
                source: VersionSource {
                    repository: opts.required("source-repository")?,
                    tag: opts.required("tag")?,
                    commit: opts.required("commit")?,
                    path: opts.value("source-path")?,
                    archive: None,
                },
                builder: opts
                    .parsed("builder")?
                    .ok_or_else(|| PublishError("--builder is required".into()))?,
                attestation: opts.value("attestation")?,
                published_at: opts.now()?,
            };
            let first_party = opts.flag("first-party");
            opts.finish()?;
            let (entry, added) = catalog::add_version(&ws, &package, info, new, first_party)?;
            format!(
                "{} {}: sha256 {}, {} bytes{}",
                entry.package.url.rsplit('/').next().unwrap_or_default(),
                entry.version,
                entry.package.sha256,
                entry.package.size,
                if added { "" } else { " (already recorded)" }
            )
        }
        "yank" => {
            let id = plugin_id(&opts.required("id")?)?;
            let version: Version = opts
                .parsed("version")?
                .ok_or_else(|| PublishError("--version is required".into()))?;
            let reason = opts.required("reason")?;
            let advisory = opts.value("advisory")?;
            let at = opts.now()?;
            opts.finish()?;
            catalog::yank(&ws, &id, &version, reason, advisory, at)?;
            format!("yanked {id} {version}")
        }
        "revoke" => {
            let revocation = Revocation {
                id: plugin_id(&opts.required("id")?)?,
                versions: versions_of(&opts.required("versions")?)?,
                action: action_of(&opts.required("action")?)?,
                reason: opts.required("reason")?,
                advisory: opts.value("advisory")?,
                at: opts.now()?,
            };
            opts.finish()?;
            let id = revocation.id.clone();
            let outcome = catalog::revoke(&ws, revocation)?;
            format!("revocation of {id} {outcome}")
        }
        "lift-revocation" => {
            let id = plugin_id(&opts.required("id")?)?;
            let versions = opts
                .value("versions")?
                .map(|v| versions_of(&v))
                .transpose()?;
            opts.finish()?;
            let removed = catalog::lift(&ws, &id, versions.as_ref())?;
            format!("removed {removed} revocation(s) of {id}")
        }
        "build-index" => {
            let options = BuildIndex {
                source_commit: opts.required("source-commit")?,
                min_app_version: opts.parsed("min-app-version")?,
                keep: opts.parsed("keep")?.unwrap_or(DEFAULT_KEEP),
                generated_at: opts.now()?,
            };
            opts.finish()?;
            let seq = index::build_index(&ws, &options)?;
            format!("wrote site/v1/index/{seq}.json; sign it with `sign-index`")
        }
        "sign-index" => {
            let seq = opts.parsed("seq")?;
            let key = signing_key(&opts, io)?;
            opts.finish()?;
            let seq = index::sign_index(&ws, seq, &key)?;
            format!("signed site/v1/index/{seq}.json")
        }
        "timestamp" => {
            let validity = opts.duration(
                "expires-in",
                TimeDelta::days(DEFAULT_TIMESTAMP_VALIDITY_DAYS),
            )?;
            let now = opts.now()?;
            let key = signing_key(&opts, io)?;
            opts.finish()?;
            let timestamp = index::timestamp(&ws, &key, now, validity)?;
            format!(
                "signed site/v1/timestamp.json: seq {}, key set {}, expires {}",
                timestamp.seq,
                timestamp.keys_version,
                crate::datetime::format(&timestamp.expires)
            )
        }
        "verify-site" => {
            let roots = opts.path("roots")?;
            let state = opts.path("state")?;
            let min_remaining = opts.duration("min-remaining", TimeDelta::zero())?;
            let require_packages = opts.flag("require-packages");
            let now = opts.now()?;
            opts.finish()?;
            let report = verify::run(
                &ws,
                roots.as_deref(),
                state.as_deref(),
                now,
                min_remaining,
                require_packages,
            )?;
            format!(
                "site accepted by the client: key set {}, seq {} ({}), expires {}, {} plugin(s), \
                 {} installable version(s), {} package(s) verified, {} revocation(s)",
                report.keys_version,
                report.seq,
                if report.updated { "new" } else { "unchanged" },
                crate::datetime::format(&report.expires),
                report.plugins,
                report.installable_versions,
                report.packages_verified,
                report.revocations
            )
        }
        "release sign" => {
            let package = std::fs::read(opts.required("package")?)
                .map_err(|e| PublishError(format!("--package: {e}")))?;
            let id = plugin_id(&opts.required("id")?)?;
            let version: Version = opts
                .parsed("version")?
                .ok_or_else(|| PublishError("--version is required".into()))?;
            let out = PathBuf::from(opts.required("out")?);
            if out.exists() {
                return fail(format!(
                    "{} already exists; refusing to overwrite",
                    out.display()
                ));
            }
            // Every check that parses the package runs before the secret
            // key is loaded, and the key is dropped before the signature is
            // verified.
            let claim = super::release::preflight(&package, id, version)?;
            let key = release_key(&opts, io)?;
            opts.finish()?;
            let signature = super::release::sign(&package, &claim, &key)?;
            let public = key.public().to_owned();
            drop(key);
            super::release::check_signed(&package, &signature, &public, &claim)?;
            std::fs::write(&out, format!("{signature}\n"))
                .map_err(|e| PublishError(format!("{}: {e}", out.display())))?;
            format!(
                "signed {} {} (sha256 {}, {} bytes) with publisher key {} to {}",
                claim.id,
                claim.version,
                claim.package.sha256,
                claim.package.size,
                crate::publisher::fingerprint(&public).unwrap_or_default(),
                out.display()
            )
        }
        "release verify" => {
            let package = PathBuf::from(opts.required("package")?);
            let signatures: Vec<PathBuf> = opts
                .all("signature")
                .into_iter()
                .map(PathBuf::from)
                .collect();
            if signatures.is_empty() {
                return fail("--signature is required (once per publisher key that signed)");
            }
            let claim = crate::publisher::ReleaseClaim {
                id: plugin_id(&opts.required("id")?)?,
                version: opts
                    .parsed("version")?
                    .ok_or_else(|| PublishError("--version is required".into()))?,
                package: deckpilot_plugin_manifest::PackageDigest {
                    sha256: crate::publisher::parse_sha256(&opts.required("sha256")?)
                        .map_err(|e| PublishError(format!("--sha256: {e}")))?,
                    size: opts
                        .parsed("size")?
                        .ok_or_else(|| PublishError("--size is required".into()))?,
                },
            };
            let platform = opts
                .value("platform")?
                .map(|p| super::release::parse_platform(&p))
                .transpose()?;
            let host = super::release::host(
                opts.parsed("app-version")?
                    .ok_or_else(|| PublishError("--app-version is required".into()))?,
                &opts.required("plugin-api")?,
                platform,
            )?;
            let keys = opts.path("keys")?;
            let released_pins = opts.path("released-pins")?;
            let revocations = opts.path("revocations")?;
            let audit = opts.path("audit")?;
            let unpack = opts.path("unpack")?;
            let now = opts.now()?;
            opts.finish()?;
            super::release::verify(&super::release::VerifyRequest {
                package: &package,
                signatures: signatures.iter().map(PathBuf::as_path).collect(),
                keys: keys.as_deref(),
                claim,
                host,
                released_pins: released_pins.as_deref(),
                revocations: revocations.as_deref(),
                audit: audit.as_deref(),
                unpack: unpack.as_deref(),
                now,
            })?
        }
        other => return fail(format!("unknown command `{other}`; see `help`")),
    };
    writeln!(io.out, "{message}").map_err(|e| PublishError(format!("stdout: {e}")))
}
