//! Every constraint of the JSON Schema plus the cross-field rules of
//! architecture section 7 that a schema cannot express.
//!
//! Serde only checks structure (fields, types, enum values). Everything
//! else — lengths, counts, patterns, uniqueness, and the rules between
//! fields — is here, so a manifest built in code is checked the same way as
//! one read from disk.

use std::collections::{BTreeSet, HashSet};
use std::fmt::Display;

use semver::Version;

use crate::compat::parse_number;
use crate::error::{Issue, IssueCode};
use crate::identity::{
    RESERVED_PUBLISHERS, is_camel_key, is_local_id, is_publisher_id, is_variable_segment,
};
use crate::model::{
    Action, Credential, CredentialScheme, Effect, Manifest, Node, Parameter, ParameterKind,
    ParameterValue, Permission, PermissionKind, ProfileScope, Setting, VariableScope,
};
use crate::{MANIFEST_VERSION, path_rules};

/// Error codes every plugin may return without declaring them (`SharedCode`
/// in the SDK). `errorCodes` must not redeclare them (section 7, rule 13).
pub const SHARED_ERROR_CODES: &[&str] = &[
    "not-permitted",
    "invalid-parameters",
    "no-active-project",
    "credential-missing",
    "reference-missing",
    "reference-type",
    "reference-denied",
    "blocked-address",
    "dns-failed",
    "connection-failed",
    "tls-failed",
    "too-many-redirects",
    "insecure-redirect",
    "insecure-credentials",
    "request-failed",
    "sensitive-header",
    "invalid-url",
    "host-not-allowed",
    "throttled",
    "storage-quota",
    "timeout",
    "unsupported",
    "plugin-error",
    "plugin-unavailable",
    "invalid-result",
];

/// The setting key the agent generates for `privateNetwork` hosts.
pub const GENERATED_SETTING_KEY: &str = "privateHosts";

#[derive(Debug, Default)]
pub(crate) struct Checker {
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
}

impl Checker {
    fn error(&mut self, path: impl Into<String>, code: IssueCode, message: impl Into<String>) {
        self.errors.push(Issue {
            path: path.into(),
            code,
            message: message.into(),
        });
    }

    fn warn(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(Issue {
            path: path.into(),
            code: IssueCode::Review,
            message: message.into(),
        });
    }

    /// Length in characters (JSON Schema counts code points).
    fn text(&mut self, path: &str, value: &str, min: usize, max: usize) {
        let len = value.chars().count();
        if len < min {
            let message = if min == 1 {
                "must not be empty".to_owned()
            } else {
                format!("must be at least {min} characters (it has {len})")
            };
            self.error(path, IssueCode::Length, message);
        } else if len > max {
            self.error(
                path,
                IssueCode::Length,
                format!("must be at most {max} characters (it has {len})"),
            );
        }
    }

    fn opt_text(&mut self, path: &str, value: Option<&String>, max: usize) {
        if let Some(value) = value {
            self.text(path, value, 0, max);
        }
    }

    fn count(&mut self, path: &str, len: usize, min: usize, max: usize) {
        if len < min {
            let message = if min == 1 {
                "must not be empty".to_owned()
            } else {
                format!("needs at least {min} entries (it has {len})")
            };
            self.error(path, IssueCode::Count, message);
        } else if len > max {
            self.error(
                path,
                IssueCode::Count,
                format!("allows at most {max} entries (it has {len})"),
            );
        }
    }

    fn range(&mut self, path: &str, value: u64, min: u64, max: u64) {
        if value < min || value > max {
            self.error(
                path,
                IssueCode::Range,
                format!("must be between {min} and {max} (it is {value})"),
            );
        }
    }

    /// Reports every repeated item at its own index.
    fn unique<T: Display>(&mut self, path: &str, items: impl IntoIterator<Item = T>) {
        let mut seen = HashSet::new();
        for (index, item) in items.into_iter().enumerate() {
            let text = item.to_string();
            if !seen.insert(text.clone()) {
                self.error(
                    format!("{path}[{index}]"),
                    IssueCode::Duplicate,
                    format!("`{text}` appears more than once"),
                );
            }
        }
    }

    fn https_url(&mut self, path: &str, value: &str) {
        if value.chars().count() > 512 {
            self.error(path, IssueCode::Length, "must be at most 512 characters");
            return;
        }
        let ok = value.starts_with("https://")
            && url::Url::parse(value)
                .is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some());
        if !ok {
            self.error(
                path,
                IssueCode::InvalidUrl,
                format!("`{value}` is not an https:// URL"),
            );
        }
    }

    fn relative_path(&mut self, path: &str, value: &str) {
        if let Err(message) = path_rules::check_relative(value) {
            self.error(path, IssueCode::InvalidPath, format!("`{value}` {message}"));
        }
    }

    #[expect(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "the schema requires a lowercase .png"
    )]
    fn png_asset(&mut self, path: &str, value: &str) {
        if let Err(message) = path_rules::check_relative(value) {
            self.error(path, IssueCode::InvalidPath, format!("`{value}` {message}"));
        } else if !value.starts_with("assets/")
            || !value.ends_with(".png")
            || value.len() <= "assets/".len() + ".png".len()
        {
            self.error(
                path,
                IssueCode::InvalidPath,
                format!("`{value}` must be a .png file under assets/"),
            );
        }
    }

    fn host(&mut self, path: &str, value: &str) {
        if let Err(message) = check_host(value) {
            self.error(path, IssueCode::InvalidHost, format!("`{value}` {message}"));
        }
    }

    fn hosts(&mut self, path: &str, hosts: &[String], max: usize) {
        self.count(path, hosts.len(), 1, max);
        for (index, host) in hosts.iter().enumerate() {
            self.host(&format!("{path}[{index}]"), host);
        }
        self.unique(path, hosts);
    }

    /// DeckPilot's release-tag grammar: no build metadata, pre-release only
    /// `alpha.N`, `beta.N` or `rc.N`.
    fn release_version(&mut self, path: &str, version: &Version) {
        if !version.build.is_empty() {
            self.error(
                path,
                IssueCode::InvalidVersion,
                format!("`{version}` has build metadata (+…), which is not allowed"),
            );
        }
        if !version.pre.is_empty() {
            let ok = version
                .pre
                .as_str()
                .split_once('.')
                .is_some_and(|(label, n)| {
                    matches!(label, "alpha" | "beta" | "rc")
                        && parse_number(n).is_some_and(|n| n >= 1)
                });
            if !ok {
                self.error(
                    path,
                    IssueCode::InvalidVersion,
                    format!(
                        "`{version}` has pre-release `{}`; only alpha.N, beta.N or rc.N (N ≥ 1) \
                         are allowed",
                        version.pre
                    ),
                );
            }
        }
        if version.to_string().len() > 32 {
            self.error(path, IssueCode::Length, "must be at most 32 characters");
        }
    }
}

/// `$defs/hostPattern`, plus a port range check the pattern cannot make.
pub(crate) fn check_host(host: &str) -> Result<(), String> {
    if host.len() > 253 {
        return Err("is longer than 253 characters".into());
    }
    if host == "*" || host == "*." {
        return Err("would allow every host; list each host or use *.<domain>".into());
    }
    if host.contains("://") || host.contains('/') {
        return Err("must be a host name, not a URL".into());
    }
    if host.contains('[') || host.contains(']') {
        return Err(
            "is an IP address; plugins name DNS hosts (private hosts come from the user via \
             privateNetwork)"
                .into(),
        );
    }
    if host.chars().any(|c| c.is_ascii_uppercase()) {
        return Err("must be lowercase".into());
    }
    let (wildcard, rest) = match host.strip_prefix("*.") {
        Some(rest) => (true, rest),
        None => (false, host),
    };
    if rest.contains('*') {
        return Err("may only use a wildcard as a leading `*.`".into());
    }
    let name = match rest.split_once(':') {
        Some((name, port)) => {
            let valid = port.len() <= 5
                && parse_number(port).is_some_and(|port| (1..=65_535).contains(&port));
            if !valid {
                return Err("has an invalid port: use 1-65535".into());
            }
            name
        }
        None => rest,
    };
    let labels: Vec<&str> = name.split('.').collect();
    if labels.iter().any(|label| label.is_empty()) {
        return Err("has an empty label".into());
    }
    if labels
        .last()
        .is_some_and(|tld| tld.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(
            "is an IP address; plugins name DNS hosts (private hosts come from the user via \
             privateNetwork)"
                .into(),
        );
    }
    if labels.len() < 2 {
        return Err(if wildcard {
            "would cover a whole top-level domain".into()
        } else if name == "localhost" {
            "is not allowed: localhost and private hosts come from the user via privateNetwork"
                .into()
        } else {
            "needs at least two labels, e.g. api.example.com".into()
        });
    }
    for label in &labels {
        let valid = label.len() <= 63
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !valid {
            return Err(format!("has an invalid label `{label}`"));
        }
    }
    if labels
        .last()
        .is_some_and(|tld| !tld.starts_with(|c: char| c.is_ascii_lowercase()))
    {
        return Err("has a top-level label that does not start with a letter".into());
    }
    Ok(())
}

/// Splits a host pattern into its name and port.
fn split_port(host: &str) -> (&str, Option<&str>) {
    match host.rsplit_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    }
}

/// Whether the host pattern `allowed` covers the host pattern `host`.
pub(crate) fn host_covers(allowed: &str, host: &str) -> bool {
    let (allowed_name, allowed_port) = split_port(allowed);
    let (name, port) = split_port(host);
    if allowed_port != port {
        return false;
    }
    if allowed_name == name {
        return true;
    }
    match allowed_name.strip_prefix("*.") {
        Some(domain) => {
            let name = name.strip_prefix("*.").unwrap_or(name);
            name.len() > domain.len()
                && name.ends_with(domain)
                && name.as_bytes()[name.len() - domain.len() - 1] == b'.'
        }
        None => false,
    }
}

pub(crate) fn check(manifest: &Manifest) -> Checker {
    let mut c = Checker::default();
    top_level(&mut c, manifest);
    permissions(&mut c, manifest);
    actions(&mut c, manifest);
    variables(&mut c, manifest);
    settings(&mut c, manifest);
    credentials(&mut c, manifest);
    error_codes(&mut c, manifest);
    needs_project(&mut c, manifest);
    c
}

fn top_level(c: &mut Checker, m: &Manifest) {
    identity(c, m);
    metadata(c, m);
    engines(c, m);
    files(c, m);
}

fn identity(c: &mut Checker, m: &Manifest) {
    if m.manifest_version != MANIFEST_VERSION {
        c.error(
            "manifestVersion",
            IssueCode::Range,
            format!("must be {MANIFEST_VERSION}"),
        );
    }
    c.opt_text("$schema", m.schema.as_ref(), 256);

    if is_publisher_id(&m.publisher.id) {
        if RESERVED_PUBLISHERS.contains(&m.publisher.id.as_str()) {
            c.error(
                "publisher.id",
                IssueCode::Reserved,
                format!("`{}` is reserved for DeckPilot", m.publisher.id),
            );
        }
        if m.id.publisher() != m.publisher.id {
            c.error(
                "id",
                IssueCode::Mismatch,
                format!(
                    "`{}` must start with the publisher id `{}` and a dot",
                    m.id, m.publisher.id
                ),
            );
        }
    } else {
        c.error(
            "publisher.id",
            IssueCode::InvalidId,
            format!(
                "`{}` is not a valid publisher id: use 2-24 lowercase letters and digits, \
                 starting with a letter, with single hyphens each followed by a letter",
                m.publisher.id
            ),
        );
    }
    c.text("publisher.name", &m.publisher.name, 1, 64);
    if let Some(url) = &m.publisher.url {
        c.https_url("publisher.url", url);
    }
}

fn metadata(c: &mut Checker, m: &Manifest) {
    c.text("name", &m.name, 1, 48);
    c.release_version("version", &m.version);
    c.text("description", &m.description, 1, 300);
    if let Some(homepage) = &m.homepage {
        c.https_url("homepage", homepage);
    }
    if let Some(repository) = &m.repository {
        if !is_github_repository(&repository.url) {
            c.error(
                "repository.url",
                IssueCode::InvalidUrl,
                format!(
                    "`{}` is not https://github.com/<owner>/<repository>",
                    repository.url
                ),
            );
        }
        if let Some(directory) = &repository.directory {
            c.relative_path("repository.directory", directory);
        }
    }
    c.text("license", &m.license, 1, 64);
    if let Some(keywords) = &m.keywords {
        c.count("keywords", keywords.len(), 0, 10);
        for (index, keyword) in keywords.iter().enumerate() {
            let valid = (2..=24).contains(&keyword.len())
                && keyword
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !valid {
                c.error(
                    format!("keywords[{index}]"),
                    IssueCode::InvalidValue,
                    format!("`{keyword}` must be 2-24 lowercase letters, digits or hyphens"),
                );
            }
        }
        c.unique("keywords", keywords);
    }
    if let Some(categories) = &m.categories {
        c.count("categories", categories.len(), 0, 3);
        c.unique("categories", categories.iter().map(|x| format!("{x:?}")));
    }
}

fn engines(c: &mut Checker, m: &Manifest) {
    if !m.engines.deckpilot.is_satisfiable() {
        c.error(
            "engines.deckpilot",
            IssueCode::UnsatisfiableRange,
            format!("`{}` matches no DeckPilot version", m.engines.deckpilot),
        );
    }
    if let Some(sdk) = &m.engines.sdk {
        c.release_version("engines.sdk", sdk);
    }
    c.relative_path("entry", &m.entry);
    if let Some(platforms) = &m.platforms {
        if platforms.is_empty() {
            c.error(
                "platforms",
                IssueCode::Count,
                "must not be empty; omit it to support every platform",
            );
        }
        c.unique("platforms", platforms);
    }
}

fn files(c: &mut Checker, m: &Manifest) {
    if let Some(icon) = &m.icons.size32 {
        c.png_asset("icons.32", icon);
    }
    c.png_asset("icons.128", &m.icons.size128);
    if let Some(icon) = &m.icons.size256 {
        c.png_asset("icons.256", icon);
    }
    if let Some(assets) = &m.assets {
        if let Some(icons) = &assets.key_icons {
            c.count("assets.keyIcons", icons.len(), 0, 64);
            for (name, file) in icons {
                let path = format!("assets.keyIcons.{name}");
                let valid = name.len() <= 48
                    && name.starts_with(|ch: char| ch.is_ascii_lowercase())
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                if !valid {
                    c.error(
                        &path,
                        IssueCode::InvalidId,
                        format!(
                            "key icon name `{name}` must be 1-48 lowercase letters, digits or \
                             hyphens, starting with a letter"
                        ),
                    );
                }
                c.png_asset(&path, file);
            }
        }
        if let Some(screenshots) = &assets.screenshots {
            c.count("assets.screenshots", screenshots.len(), 0, 8);
            for (index, file) in screenshots.iter().enumerate() {
                c.png_asset(&format!("assets.screenshots[{index}]"), file);
            }
        }
        for (field, value, expected) in [
            ("assets.readme", &assets.readme, "README.md"),
            ("assets.changelog", &assets.changelog, "CHANGELOG.md"),
        ] {
            if value.as_deref().is_some_and(|v| v != expected) {
                c.error(
                    field,
                    IssueCode::InvalidValue,
                    format!("must be `{expected}`"),
                );
            }
        }
    }
    if let Some(device) = m.supports.as_ref().and_then(|s| s.device.as_ref()) {
        for (field, families) in [
            ("supports.device.requires", &device.requires),
            ("supports.device.uses", &device.uses),
        ] {
            if let Some(families) = families {
                c.unique(field, families.iter().map(|x| format!("{x:?}")));
            }
        }
    }
}

fn is_github_repository(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://github.com/") else {
        return false;
    };
    let Some((owner, repository)) = rest.split_once('/') else {
        return false;
    };
    url.len() <= 200
        && (1..=39).contains(&owner.len())
        && owner.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && (1..=100).contains(&repository.len())
        && repository
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        // GitHub has no repositories named `.` or `..`; the schema pattern
        // allows them.
        && repository != "."
        && repository != ".."
}

fn permissions(c: &mut Checker, m: &Manifest) {
    c.count("permissions", m.permissions.len(), 0, 16);
    let own_namespace = m.id.variable_namespace();
    let mut kinds = BTreeSet::new();
    for (index, permission) in m.permissions.iter().enumerate() {
        let path = format!("permissions[{index}]");
        let kind = permission.kind();
        if !kinds.insert(kind) {
            c.error(
                &path,
                IssueCode::Duplicate,
                format!(
                    "only one `{}` permission is allowed; merge them",
                    kind.as_str()
                ),
            );
        }
        c.text(
            &format!("{path}.justification"),
            permission.justification(),
            10,
            200,
        );
        match permission {
            Permission::Network { hosts, methods, .. } => {
                c.hosts(&format!("{path}.hosts"), hosts, 16);
                if let Some(methods) = methods {
                    c.count(&format!("{path}.methods"), methods.len(), 1, usize::MAX);
                    c.unique(
                        &format!("{path}.methods"),
                        methods.iter().map(|x| format!("{x:?}")),
                    );
                }
            }
            Permission::Credentials { refs, .. } => {
                let field = format!("{path}.refs");
                c.count(&field, refs.len(), 1, 8);
                for (i, key) in refs.iter().enumerate() {
                    camel_key(c, &format!("{field}[{i}]"), key);
                }
                c.unique(&field, refs);
            }
            Permission::Profile { read, .. } => {
                c.count(&format!("{path}.read"), read.len(), 1, usize::MAX);
                c.unique(
                    &format!("{path}.read"),
                    read.iter().map(|x| format!("{x:?}")),
                );
            }
            Permission::Variables { read, .. } => {
                let field = format!("{path}.read");
                c.count(&field, read.len(), 1, 32);
                for (i, entry) in read.iter().enumerate() {
                    variable_read(c, &format!("{field}[{i}]"), entry, &own_namespace);
                }
                c.unique(&field, read);
            }
            Permission::Storage { quota_kb, .. } => {
                if !(257..=10_240).contains(quota_kb) {
                    c.error(
                        format!("{path}.quotaKb"),
                        IssueCode::Range,
                        format!(
                            "must be between 257 and 10240 (it is {quota_kb}); the first 256 KiB \
                             need no permission"
                        ),
                    );
                }
            }
            Permission::OpenUrl { hosts, .. } => c.hosts(&format!("{path}.hosts"), hosts, 16),
            Permission::Background {
                min_interval_seconds,
                ..
            } => c.range(
                &format!("{path}.minIntervalSeconds"),
                *min_interval_seconds,
                15,
                86_400,
            ),
            Permission::Device { state, icons, .. } => {
                if state != &Some(true) && icons != &Some(true) {
                    c.warn(&path, "grants nothing: set `state` or `icons` to true");
                }
            }
            Permission::Notifications { .. } => {}
        }
    }
}

/// `variables.read` entries (section 7, rule 10).
fn variable_read(c: &mut Checker, path: &str, entry: &str, own_namespace: &str) {
    let mut fail = |message: String| c.error(path, IssueCode::InvalidVariable, message);
    if entry.chars().count() > 200 {
        return fail("must be at most 200 characters".into());
    }
    let (base, wildcard) = match entry.strip_suffix(".*") {
        Some(base) => (base, true),
        None => (entry, false),
    };
    let segments: Vec<&str> = base.split('.').collect();
    if !(2..=6).contains(&segments.len()) {
        return fail(format!(
            "`{entry}` needs 2 to 6 segments (a final `.*` allowed), e.g. shared.paths.root"
        ));
    }
    if let Some(bad) = segments.iter().find(|s| !is_variable_segment(s)) {
        return fail(format!(
            "`{entry}` has an invalid segment `{bad}`: wildcards are allowed only as a final `.*`"
        ));
    }
    match segments[0] {
        "steps" => fail(format!(
            "`{entry}`: workflow step outputs are never readable"
        )),
        "plugins" if base == own_namespace || base.starts_with(&format!("{own_namespace}.")) => {
            fail(format!(
                "`{entry}`: the plugin's own variables are always readable; do not list them"
            ));
        }
        "plugins" if wildcard => fail(format!(
            "`{entry}`: no wildcards under plugins; list each variable of another plugin by its \
             exact id"
        )),
        "plugins" if segments.len() < 4 => fail(format!(
            "`{entry}`: name one variable of another plugin by its exact id \
             (plugins.<publisher>.<name>.<variable>)"
        )),
        _ => {}
    }
}

fn camel_key(c: &mut Checker, path: &str, key: &str) {
    if !is_camel_key(key) {
        c.error(
            path,
            IssueCode::InvalidId,
            format!(
                "`{key}` must be lowerCamelCase: a lowercase letter, then up to 31 letters or \
                 digits"
            ),
        );
    }
}

fn parameter_kind(c: &mut Checker, path: &str, kind: &ParameterKind) {
    let max_len = |c: &mut Checker, field: &str, value: Option<u64>, max: u64| {
        if let Some(value) = value {
            c.range(&format!("{path}.{field}"), value, 1, max);
        }
    };
    match kind {
        ParameterKind::Text { max_length, .. } | ParameterKind::Code { max_length, .. } => {
            max_len(c, "maxLength", *max_length, 100_000);
        }
        ParameterKind::Integer {
            min: Some(min),
            max: Some(max),
        } if min > max => c.error(
            path,
            IssueCode::Conflict,
            format!("min ({min}) is greater than max ({max})"),
        ),
        ParameterKind::Choice { options } => {
            let field = format!("{path}.options");
            c.count(&field, options.len(), 1, 64);
            for (i, option) in options.iter().enumerate() {
                c.text(&format!("{field}[{i}]"), option, 1, 64);
            }
            c.unique(&field, options);
        }
        ParameterKind::Lines {
            item_label,
            max_items,
            max_length,
        } => {
            c.opt_text(&format!("{path}.itemLabel"), item_label.as_ref(), 32);
            max_len(c, "maxItems", *max_items, 1000);
            max_len(c, "maxLength", *max_length, 10_000);
        }
        ParameterKind::Pairs {
            name_label,
            value_label,
            max_items,
            ..
        } => {
            c.opt_text(&format!("{path}.nameLabel"), name_label.as_ref(), 32);
            c.opt_text(&format!("{path}.valueLabel"), value_label.as_ref(), 32);
            max_len(c, "maxItems", *max_items, 200);
        }
        ParameterKind::Integer { .. } | ParameterKind::Boolean {} | ParameterKind::Project {} => {}
    }
}

fn kind_name(kind: &ParameterKind) -> &'static str {
    match kind {
        ParameterKind::Text { .. } => "text",
        ParameterKind::Integer { .. } => "integer",
        ParameterKind::Boolean {} => "boolean",
        ParameterKind::Choice { .. } => "choice",
        ParameterKind::Project {} => "project",
        ParameterKind::Lines { .. } => "lines",
        ParameterKind::Pairs { .. } => "pairs",
        ParameterKind::Code { .. } => "code",
    }
}

fn too_long(len: usize, max: Option<u64>) -> bool {
    max.is_some_and(|max| u64::try_from(len).unwrap_or(u64::MAX) > max)
}

/// Rule 9: a default must pass normalization for its kind.
fn default_value(c: &mut Checker, path: &str, kind: &ParameterKind, value: &ParameterValue) {
    let problem = match (kind, value) {
        (
            ParameterKind::Text { max_length, .. } | ParameterKind::Code { max_length, .. },
            ParameterValue::Text(text),
        ) => too_long(text.chars().count(), *max_length)
            .then(|| "is longer than the parameter's maxLength".to_owned()),
        (
            ParameterKind::Lines {
                max_items,
                max_length,
                ..
            },
            ParameterValue::Text(text),
        ) => {
            let items: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            if too_long(items.len(), *max_items) {
                Some("has more lines than the parameter's maxItems".to_owned())
            } else if items
                .iter()
                .any(|item| too_long(item.chars().count(), *max_length))
            {
                Some("has a line longer than the parameter's maxLength".to_owned())
            } else {
                None
            }
        }
        (
            ParameterKind::Pairs {
                separator,
                max_items,
                ..
            },
            ParameterValue::Text(text),
        ) => {
            let separator = match separator {
                crate::model::PairSeparator::Equals => '=',
                crate::model::PairSeparator::Colon => ':',
            };
            let rows: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            if too_long(rows.len(), *max_items) {
                Some("has more rows than the parameter's maxItems".to_owned())
            } else if rows.iter().any(|row| !row.contains(separator)) {
                Some(format!("has a row without the separator `{separator}`"))
            } else {
                None
            }
        }
        (ParameterKind::Integer { min, max }, ParameterValue::Integer(n)) => {
            (min.is_some_and(|min| *n < min) || max.is_some_and(|max| *n > max))
                .then(|| format!("{n} is outside the parameter's min/max"))
        }
        (ParameterKind::Boolean {}, ParameterValue::Boolean(_)) => None,
        (ParameterKind::Choice { options }, ParameterValue::Choice(choice)) => {
            (!options.contains(choice)).then(|| format!("`{choice}` is not one of the options"))
        }
        (ParameterKind::Project {}, _) => Some("a project parameter cannot have a default".into()),
        (kind, value) => {
            let value = match value {
                ParameterValue::Text(_) => "text",
                ParameterValue::Integer(_) => "integer",
                ParameterValue::Boolean(_) => "boolean",
                ParameterValue::Choice(_) => "choice",
            };
            Some(format!(
                "a {value} default does not fit a {} parameter",
                kind_name(kind)
            ))
        }
    };
    if let Some(problem) = problem {
        c.error(path, IssueCode::InvalidDefault, problem);
    }
}

struct FieldRef<'a> {
    key: &'a str,
    label: &'a str,
    description: Option<&'a String>,
    kind: &'a ParameterKind,
    default: Option<&'a ParameterValue>,
}

impl<'a> From<&'a Parameter> for FieldRef<'a> {
    fn from(p: &'a Parameter) -> Self {
        Self {
            key: &p.key,
            label: &p.label,
            description: p.description.as_ref(),
            kind: &p.kind,
            default: p.default.as_ref(),
        }
    }
}

impl<'a> From<&'a Setting> for FieldRef<'a> {
    fn from(p: &'a Setting) -> Self {
        Self {
            key: &p.key,
            label: &p.label,
            description: p.description.as_ref(),
            kind: &p.kind,
            default: p.default.as_ref(),
        }
    }
}

fn field(c: &mut Checker, path: &str, f: &FieldRef<'_>) {
    camel_key(c, &format!("{path}.key"), f.key);
    c.text(&format!("{path}.label"), f.label, 1, 48);
    c.opt_text(&format!("{path}.description"), f.description, 300);
    parameter_kind(c, &format!("{path}.kind"), f.kind);
    if let Some(default) = f.default {
        default_value(c, &format!("{path}.default"), f.kind, default);
    }
}

fn actions(c: &mut Checker, m: &Manifest) {
    let actions = m.contributes.actions();
    c.count("contributes.actions", actions.len(), 0, 64);
    for (index, action) in actions.iter().enumerate() {
        let path = format!("contributes.actions[{index}]");
        if !is_local_id(&action.id, 48) {
            c.error(
                format!("{path}.id"),
                IssueCode::InvalidId,
                format!(
                    "`{}` must be 1-48 lowercase letters and digits, starting with a letter, \
                     with single hyphens",
                    action.id
                ),
            );
        }
        c.text(&format!("{path}.name"), &action.name, 1, 48);
        c.text(&format!("{path}.description"), &action.description, 1, 300);
        c.text(&format!("{path}.category"), &action.category, 1, 32);
        c.count(
            &format!("{path}.parameters"),
            action.parameters.len(),
            0,
            32,
        );
        for (i, parameter) in action.parameters.iter().enumerate() {
            field(c, &format!("{path}.parameters[{i}]"), &parameter.into());
        }
        c.unique(
            &format!("{path}.parameters"),
            action.parameters.iter().map(|p| &p.key),
        );
        replaced_by(c, &path, action, actions);
        if let Some(node) = &action.node {
            self::node(c, &format!("{path}.node"), node);
        }
        if let Some(state) = &action.state {
            c.range(
                &format!("{path}.state.intervalSeconds"),
                state.interval_seconds,
                1,
                86_400,
            );
            // Rule 6.
            if action
                .node
                .as_ref()
                .is_some_and(|n| n.effect() != Effect::ReadOnly)
            {
                c.error(
                    format!("{path}.state"),
                    IssueCode::Conflict,
                    "button state needs a readOnly action (node.effect)",
                );
            }
        }
        if let Some(requires) = action.device.as_ref().and_then(|d| d.requires.as_ref()) {
            c.unique(
                &format!("{path}.device.requires"),
                requires.iter().map(|x| format!("{x:?}")),
            );
        }
    }
    c.unique("contributes.actions", actions.iter().map(|a| &a.id));
}

/// Rule 7.
fn replaced_by(c: &mut Checker, path: &str, action: &Action, actions: &[Action]) {
    let Some(target) = &action.replaced_by else {
        return;
    };
    let field = format!("{path}.replacedBy");
    if !is_local_id(target, 48) {
        c.error(
            &field,
            IssueCode::InvalidId,
            format!("`{target}` is not an action id"),
        );
    } else if target == &action.id {
        c.error(
            &field,
            IssueCode::Conflict,
            "an action cannot replace itself",
        );
    } else if !actions.iter().any(|a| &a.id == target) {
        c.error(
            &field,
            IssueCode::UnknownReference,
            format!("`{target}` is not an action of this plugin"),
        );
    }
    if action.deprecated != Some(true) {
        c.error(
            &field,
            IssueCode::Conflict,
            "replacedBy needs `deprecated: true`",
        );
    }
}

fn node(c: &mut Checker, path: &str, node: &Node) {
    if node.version == Some(0) {
        c.error(
            format!("{path}.version"),
            IssueCode::Range,
            "must be at least 1",
        );
    }
    if let Some(icon) = &node.icon {
        let valid = (1..=64).contains(&icon.len())
            && icon
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid {
            c.error(
                format!("{path}.icon"),
                IssueCode::InvalidId,
                format!("`{icon}` must be 1-64 lowercase letters, digits or hyphens"),
            );
        }
    }
    if let Some(outputs) = &node.outputs {
        let field = format!("{path}.outputs");
        c.count(&field, outputs.len(), 0, 32);
        for (i, output) in outputs.iter().enumerate() {
            camel_key(c, &format!("{field}[{i}].key"), &output.key);
            c.text(&format!("{field}[{i}].label"), &output.label, 1, 48);
        }
        c.unique(&field, outputs.iter().map(|o| &o.key));
    }
    if let Some(timeout) = node.timeout_ms {
        c.range(&format!("{path}.timeoutMs"), timeout, 1000, 600_000);
    }
    if node.retry_safe == Some(true) && node.effect() == Effect::Modifies {
        c.warn(
            format!("{path}.retrySafe"),
            "an action that modifies something is rarely safe to retry",
        );
    }
}

fn variables(c: &mut Checker, m: &Manifest) {
    let variables = m.contributes.variables();
    c.count("contributes.variables", variables.len(), 0, 64);
    for (index, variable) in variables.iter().enumerate() {
        let path = format!("contributes.variables[{index}]");
        let segments: Vec<&str> = variable.id.split('.').collect();
        let valid = variable.id.len() <= 100
            && (1..=3).contains(&segments.len())
            && segments.iter().all(|s| is_variable_segment(s));
        if !valid {
            c.error(
                format!("{path}.id"),
                IssueCode::InvalidId,
                format!(
                    "`{}` must be 1-3 dot-separated segments, each a lowercase letter then up \
                     to 31 letters, digits or underscores",
                    variable.id
                ),
            );
        }
        c.text(&format!("{path}.name"), &variable.name, 1, 48);
        c.opt_text(
            &format!("{path}.description"),
            variable.description.as_ref(),
            300,
        );
        if let Some(requires) = &variable.requires {
            c.unique(
                &format!("{path}.requires"),
                requires.iter().map(|x| format!("{x:?}")),
            );
        }
    }
    c.unique("contributes.variables", variables.iter().map(|v| &v.id));
}

fn settings(c: &mut Checker, m: &Manifest) {
    let fields = m.contributes.settings();
    c.count("contributes.settings.fields", fields.len(), 0, 32);
    for (index, setting) in fields.iter().enumerate() {
        let path = format!("contributes.settings.fields[{index}]");
        field(c, &path, &setting.into());
        if setting.key == GENERATED_SETTING_KEY {
            c.error(
                format!("{path}.key"),
                IssueCode::Reserved,
                format!("`{GENERATED_SETTING_KEY}` is generated by DeckPilot for privateNetwork"),
            );
        }
    }
    c.unique("contributes.settings.fields", fields.iter().map(|s| &s.key));
}

fn network_hosts(m: &Manifest) -> Option<&[String]> {
    m.permissions.iter().find_map(|p| match p {
        Permission::Network { hosts, .. } => Some(hosts.as_slice()),
        _ => None,
    })
}

/// Slot fields and rule 4.
fn credentials(c: &mut Checker, m: &Manifest) {
    let slots = m.contributes.credentials();
    c.count("contributes.credentials", slots.len(), 0, 8);
    let network = network_hosts(m);
    let refs: Option<(usize, &[String])> =
        m.permissions.iter().enumerate().find_map(|(i, p)| match p {
            Permission::Credentials { refs, .. } => Some((i, refs.as_slice())),
            _ => None,
        });
    for (index, slot) in slots.iter().enumerate() {
        let path = format!("contributes.credentials[{index}]");
        credential_fields(c, &path, slot);
        if !refs.is_some_and(|(_, refs)| refs.contains(&slot.key)) {
            c.error(
                format!("{path}.key"),
                IssueCode::MissingPermission,
                format!(
                    "credential slot `{}` is not listed in a `credentials` permission's refs",
                    slot.key
                ),
            );
        }
        for (i, host) in slot.hosts.iter().enumerate() {
            if !network.is_some_and(|allowed| allowed.iter().any(|a| host_covers(a, host))) {
                c.error(
                    format!("{path}.hosts[{i}]"),
                    IssueCode::MissingPermission,
                    format!("`{host}` is not covered by the `network` permission's hosts"),
                );
            }
        }
    }
    c.unique("contributes.credentials", slots.iter().map(|s| &s.key));
    if let Some((index, refs)) = refs {
        for (i, key) in refs.iter().enumerate() {
            if !slots.iter().any(|slot| &slot.key == key) {
                c.error(
                    format!("permissions[{index}].refs[{i}]"),
                    IssueCode::UnknownReference,
                    format!("`{key}` is not a slot in contributes.credentials"),
                );
            }
        }
    }
}

fn credential_fields(c: &mut Checker, path: &str, slot: &Credential) {
    camel_key(c, &format!("{path}.key"), &slot.key);
    c.text(&format!("{path}.label"), &slot.label, 1, 48);
    c.opt_text(
        &format!("{path}.description"),
        slot.description.as_ref(),
        300,
    );
    c.hosts(&format!("{path}.hosts"), &slot.hosts, 8);
    c.count(
        &format!("{path}.schemes"),
        slot.schemes.len(),
        1,
        usize::MAX,
    );
    c.unique(
        &format!("{path}.schemes"),
        slot.schemes.iter().map(|x| format!("{x:?}")),
    );
    let name_ok = |name: &str, extra: &[u8]| {
        (1..=64).contains(&name.len())
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || extra.contains(&b))
    };
    for (scheme, field, value, extra) in [
        (
            CredentialScheme::Header,
            "headerName",
            &slot.header_name,
            &b""[..],
        ),
        (
            CredentialScheme::Query,
            "queryName",
            &slot.query_name,
            &b"_."[..],
        ),
    ] {
        match value {
            None if slot.schemes.contains(&scheme) => c.error(
                format!("{path}.{field}"),
                IssueCode::Conflict,
                format!("the `{scheme:?}` scheme needs {field}").to_lowercase(),
            ),
            Some(name) if !name_ok(name, extra) => c.error(
                format!("{path}.{field}"),
                IssueCode::InvalidValue,
                format!("`{name}` is not a valid {field}"),
            ),
            _ => {}
        }
    }
    if let Some(url) = &slot.setup_url {
        c.https_url(&format!("{path}.setupUrl"), url);
    }
}

fn error_codes(c: &mut Checker, m: &Manifest) {
    let Some(codes) = &m.contributes.error_codes else {
        return;
    };
    c.count("contributes.errorCodes", codes.len(), 0, 64);
    for (code, message) in codes {
        let path = format!("contributes.errorCodes.{code}");
        if !is_local_id(code, 64) {
            c.error(
                &path,
                IssueCode::InvalidId,
                format!(
                    "`{code}` must be lowercase letters and digits, starting with a letter, with \
                     single hyphens"
                ),
            );
        } else if SHARED_ERROR_CODES.contains(&code.as_str()) {
            c.error(
                &path,
                IssueCode::Reserved,
                format!("`{code}` is a shared code; do not redeclare it"),
            );
        }
        c.text(&path, message, 1, 200);
    }
}

/// Rule 3: project features need `profile` with `read` including `project`.
fn needs_project(c: &mut Checker, m: &Manifest) {
    let granted = m.permissions.iter().any(
        |p| matches!(p, Permission::Profile { read, .. } if read.contains(&ProfileScope::Project)),
    );
    if granted {
        return;
    }
    let mut needs = Vec::new();
    for (i, action) in m.contributes.actions().iter().enumerate() {
        if action.requires_project == Some(true) {
            needs.push(format!("contributes.actions[{i}].requiresProject"));
        }
        for (j, parameter) in action.parameters.iter().enumerate() {
            if parameter.kind == (ParameterKind::Project {}) {
                needs.push(format!("contributes.actions[{i}].parameters[{j}].kind"));
            }
        }
    }
    for (i, setting) in m.contributes.settings().iter().enumerate() {
        if setting.kind == (ParameterKind::Project {}) {
            needs.push(format!("contributes.settings.fields[{i}].kind"));
        }
    }
    for (i, variable) in m.contributes.variables().iter().enumerate() {
        if variable.scope == VariableScope::Project {
            needs.push(format!("contributes.variables[{i}].scope"));
        }
    }
    for path in needs {
        c.error(
            path,
            IssueCode::MissingPermission,
            format!(
                "needs a `{}` permission whose read includes \"project\"",
                PermissionKind::Profile.as_str()
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        for good in [
            "api.github.com",
            "*.example.com",
            "*.co.uk",
            "example.com:8443",
            "a-b.example.io",
        ] {
            assert_eq!(check_host(good), Ok(()), "{good}");
        }
        for bad in [
            "*",
            "*.com",
            "localhost",
            "intranet",
            "127.0.0.1",
            "[::1]",
            "https://api.github.com",
            "api.github.com/v3",
            "API.github.com",
            "api.*.com",
            "*.*.example.com",
            "-a.example.com",
            "example.com:0",
            "example.com:65536",
            "example.com:",
            "example..com",
            "example.1com",
        ] {
            assert!(check_host(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn coverage() {
        assert!(host_covers("api.github.com", "api.github.com"));
        assert!(host_covers("*.github.com", "api.github.com"));
        assert!(host_covers("*.github.com", "*.api.github.com"));
        assert!(!host_covers("*.github.com", "github.com"));
        assert!(!host_covers("*.github.com", "evilgithub.com"));
        assert!(!host_covers("api.github.com", "*.github.com"));
        assert!(!host_covers("api.github.com", "api.github.com:8443"));
    }
}
