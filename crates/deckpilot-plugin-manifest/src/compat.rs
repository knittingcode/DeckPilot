//! Compatibility constraints: `engines.deckpilot`, `engines.pluginApi` and
//! `platforms` (architecture sections 6.5 and 12.3 step 2).

use std::cmp::Ordering;
use std::fmt;

use semver::{BuildMetadata, Version};
use serde::{Deserialize, Serialize};

/// Longest `engines.deckpilot` text.
pub const MAX_RANGE_LEN: usize = 64;

/// An operating system a plugin can run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

impl Platform {
    pub const ALL: [Self; 3] = [Self::Macos, Self::Windows, Self::Linux];

    /// The platform this build runs on, if it is one plugins support.
    pub fn current() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::Macos)
        } else if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Macos => "macOS",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
        })
    }
}

/// Why a version range or API requirement was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RangeError {
    #[error("`{0}` is longer than 64 characters")]
    TooLong(String),
    #[error(
        "`{0}` is not a DeckPilot version range: use one comparator, or a lower and an upper bound \
         separated by one space, e.g. \">=0.8.0 <1.0.0\""
    )]
    Shape(String),
    #[error(
        "`{0}` is not a comparator: use >=, >, <=, < or = followed by MAJOR.MINOR.PATCH without \
         leading zeros or pre-release"
    )]
    Comparator(String),
    #[error("`{0}` is not a plugin API requirement: use ^MAJOR.MINOR, e.g. \"^1.0\"")]
    Api(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Exact,
    Greater,
    GreaterEq,
    Less,
    LessEq,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Comparator {
    op: Op,
    version: Version,
}

impl Comparator {
    fn parse(text: &str) -> Option<Self> {
        let (op, rest) = if let Some(rest) = text.strip_prefix(">=") {
            (Op::GreaterEq, rest)
        } else if let Some(rest) = text.strip_prefix("<=") {
            (Op::LessEq, rest)
        } else if let Some(rest) = text.strip_prefix('>') {
            (Op::Greater, rest)
        } else if let Some(rest) = text.strip_prefix('<') {
            (Op::Less, rest)
        } else if let Some(rest) = text.strip_prefix('=') {
            (Op::Exact, rest)
        } else {
            // A bare version means exactly that version, as in npm. (Cargo
            // would read it as a caret requirement; the schema says
            // comparators, so the strict reading is the safe one.)
            (Op::Exact, text)
        };
        Some(Self {
            op,
            version: parse_release(rest)?,
        })
    }

    fn matches(&self, version: &Version) -> bool {
        let ordering = version.cmp(&self.version);
        match self.op {
            Op::Exact => ordering == Ordering::Equal,
            Op::Greater => ordering == Ordering::Greater,
            Op::GreaterEq => ordering != Ordering::Less,
            Op::Less => ordering == Ordering::Less,
            Op::LessEq => ordering != Ordering::Greater,
        }
    }

    fn is_lower(&self) -> bool {
        matches!(self.op, Op::Greater | Op::GreaterEq)
    }

    fn is_upper(&self) -> bool {
        matches!(self.op, Op::Less | Op::LessEq)
    }
}

impl fmt::Display for Comparator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = match self.op {
            Op::Exact => "=",
            Op::Greater => ">",
            Op::GreaterEq => ">=",
            Op::Less => "<",
            Op::LessEq => "<=",
        };
        write!(f, "{op}{}", self.version)
    }
}

/// A non-negative decimal without leading zeros.
pub(crate) fn parse_number(text: &str) -> Option<u64> {
    if text.is_empty()
        || !text.bytes().all(|b| b.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    text.parse().ok()
}

/// `MAJOR.MINOR.PATCH`, no pre-release, no build metadata.
fn parse_release(text: &str) -> Option<Version> {
    let mut parts = text.split('.');
    let (Some(major), Some(minor), Some(patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    Some(Version::new(
        parse_number(major)?,
        parse_number(minor)?,
        parse_number(patch)?,
    ))
}

/// The DeckPilot versions a plugin runs on (`engines.deckpilot`): one
/// comparator, or a lower and an upper bound separated by one space.
///
/// Matching uses semantic-version precedence, including for pre-release
/// builds of DeckPilot: `0.9.0-alpha.1` satisfies `>=0.8.0 <1.0.0`, and
/// `0.8.0-beta.1` does not satisfy `>=0.8.0` (it precedes 0.8.0). This
/// differs on purpose from the `semver` crate's `VersionReq`, which never
/// matches a pre-release against a comparator without one, and would make
/// every plugin incompatible with every alpha build. Build metadata is
/// ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VersionRange {
    text: String,
    comparators: Vec<Comparator>,
}

impl VersionRange {
    pub fn parse(text: &str) -> Result<Self, RangeError> {
        if text.chars().count() > MAX_RANGE_LEN {
            return Err(RangeError::TooLong(text.to_owned()));
        }
        let parts: Vec<&str> = text.split(' ').collect();
        if parts.is_empty() || parts.len() > 2 || parts.iter().any(|part| part.is_empty()) {
            return Err(RangeError::Shape(text.to_owned()));
        }
        let mut comparators = Vec::with_capacity(parts.len());
        for part in &parts {
            comparators.push(
                Comparator::parse(part)
                    .ok_or_else(|| RangeError::Comparator((*part).to_owned()))?,
            );
        }
        if let [first, second] = comparators.as_slice() {
            // The schema allows `=x` or a bare version only first; this also
            // refuses two bounds in the same direction (">=1.0.0 >=2.0.0"),
            // which is never what an author means.
            let bounded =
                (first.is_lower() && second.is_upper()) || (first.is_upper() && second.is_lower());
            if !bounded {
                return Err(RangeError::Shape(text.to_owned()));
            }
        }
        Ok(Self {
            text: text.to_owned(),
            comparators,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether `version` is in the range. Build metadata is ignored.
    pub fn matches(&self, version: &Version) -> bool {
        let mut version = version.clone();
        version.build = BuildMetadata::EMPTY;
        self.comparators.iter().all(|c| c.matches(&version))
    }

    /// Whether any version can satisfy the range (`>=1.0.0 <1.0.0` cannot).
    pub fn is_satisfiable(&self) -> bool {
        let lower = self.comparators.iter().find(|c| c.is_lower());
        let upper = self.comparators.iter().find(|c| c.is_upper());
        match (lower, upper) {
            (Some(lower), Some(upper)) => match lower.version.cmp(&upper.version) {
                Ordering::Less => true,
                Ordering::Equal => lower.op == Op::GreaterEq && upper.op == Op::LessEq,
                Ordering::Greater => false,
            },
            // `<0.0.0` excludes every release DeckPilot can have.
            (None, Some(upper)) => {
                !(upper.op == Op::Less && upper.version == Version::new(0, 0, 0))
            }
            _ => true,
        }
    }
}

impl TryFrom<String> for VersionRange {
    type Error = RangeError;

    fn try_from(text: String) -> Result<Self, RangeError> {
        Self::parse(&text)
    }
}

impl From<VersionRange> for String {
    fn from(range: VersionRange) -> Self {
        range.text
    }
}

impl fmt::Display for VersionRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// A plugin API level, `MAJOR.MINOR` (architecture section 6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApiLevel {
    pub major: u64,
    pub minor: u64,
}

impl ApiLevel {
    pub const fn new(major: u64, minor: u64) -> Self {
        Self { major, minor }
    }

    /// `MAJOR.MINOR` without leading zeros.
    pub fn parse(text: &str) -> Option<Self> {
        let (major, minor) = text.split_once('.')?;
        Some(Self::new(parse_number(major)?, parse_number(minor)?))
    }
}

impl fmt::Display for ApiLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// `engines.pluginApi`: `^MAJOR.MINOR`, satisfied by the same major and at
/// least this minor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ApiRequirement {
    pub minimum: ApiLevel,
}

impl ApiRequirement {
    pub fn parse(text: &str) -> Result<Self, RangeError> {
        text.strip_prefix('^')
            .and_then(ApiLevel::parse)
            .map(|minimum| Self { minimum })
            .ok_or_else(|| RangeError::Api(text.to_owned()))
    }

    pub fn is_satisfied_by(&self, host: ApiLevel) -> bool {
        host.major == self.minimum.major && host.minor >= self.minimum.minor
    }
}

impl TryFrom<String> for ApiRequirement {
    type Error = RangeError;

    fn try_from(text: String) -> Result<Self, RangeError> {
        Self::parse(&text)
    }
}

impl From<ApiRequirement> for String {
    fn from(requirement: ApiRequirement) -> Self {
        requirement.to_string()
    }
}

impl fmt::Display for ApiRequirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "^{}", self.minimum)
    }
}

/// What a DeckPilot build offers plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEnvironment {
    pub app_version: Version,
    pub plugin_api: ApiLevel,
    pub platform: Platform,
}

/// One reason a plugin cannot run on a host, worded for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Incompatibility {
    #[error("needs DeckPilot {required}; this is DeckPilot {actual}")]
    AppVersion {
        required: VersionRange,
        actual: Version,
    },
    #[error("needs plugin API {required}; this DeckPilot provides plugin API {actual}")]
    PluginApi {
        required: ApiRequirement,
        actual: ApiLevel,
    },
    #[error("is not available for {actual} (only for {})", list(.supported))]
    Platform {
        supported: Vec<Platform>,
        actual: Platform,
    },
}

fn list(platforms: &[Platform]) -> String {
    platforms
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn ranges_match_by_precedence() {
        let range = VersionRange::parse(">=0.8.0 <1.0.0").unwrap();
        assert!(range.matches(&v("0.8.0")));
        assert!(range.matches(&v("0.9.3")));
        assert!(range.matches(&v("0.9.0-alpha.1")));
        assert!(range.matches(&v("0.9.0+build.5")));
        assert!(!range.matches(&v("0.8.0-beta.1")));
        assert!(!range.matches(&v("1.0.0")));
        assert!(range.matches(&v("1.0.0-rc.1")));
        assert!(!range.matches(&v("0.7.9")));

        let exact = VersionRange::parse("0.8.0").unwrap();
        assert!(exact.matches(&v("0.8.0")));
        assert!(!exact.matches(&v("0.8.1")));
        assert!(VersionRange::parse("<=1.2.3").unwrap().matches(&v("1.2.3")));
        assert!(!VersionRange::parse(">1.2.3").unwrap().matches(&v("1.2.3")));
        assert!(
            VersionRange::parse("<1.0.0 >=0.5.0")
                .unwrap()
                .matches(&v("0.5.0"))
        );
    }

    #[test]
    fn malformed_ranges() {
        for bad in [
            "",
            " >=1.0.0",
            ">=1.0.0  <2.0.0",
            ">=1.0.0 <2.0.0 <3.0.0",
            ">=1.0",
            ">=01.0.0",
            ">=1.0.0-alpha.1",
            "^1.0.0",
            "~1.0.0",
            ">=1.0.0 >=2.0.0",
            "<1.0.0 <=2.0.0",
            ">=1.0.0 =2.0.0",
            "=1.0.0 <2.0.0",
            "*",
            ">= 1.0.0",
            ">=1.0.0,<2.0.0",
        ] {
            assert!(VersionRange::parse(bad).is_err(), "{bad:?}");
        }
        assert!(matches!(
            VersionRange::parse(&format!(">={}.0.0", "1".repeat(70))),
            Err(RangeError::TooLong(_))
        ));
    }

    #[test]
    fn satisfiability() {
        for (text, satisfiable) in [
            (">=0.8.0 <1.0.0", true),
            (">=1.0.0 <1.0.0", false),
            (">=1.0.0 <=1.0.0", true),
            (">1.0.0 <=1.0.0", false),
            (">2.0.0 <1.0.0", false),
            ("<0.0.0", false),
            ("<=0.0.0", true),
            (">=5.0.0", true),
        ] {
            assert_eq!(
                VersionRange::parse(text).unwrap().is_satisfiable(),
                satisfiable,
                "{text}"
            );
        }
    }

    #[test]
    fn api_requirements() {
        let requirement = ApiRequirement::parse("^1.2").unwrap();
        assert!(requirement.is_satisfied_by(ApiLevel::new(1, 2)));
        assert!(requirement.is_satisfied_by(ApiLevel::new(1, 5)));
        assert!(!requirement.is_satisfied_by(ApiLevel::new(1, 1)));
        assert!(!requirement.is_satisfied_by(ApiLevel::new(2, 2)));
        assert_eq!(requirement.to_string(), "^1.2");
        for bad in ["1.0", "^1", "^1.0.0", "^01.0", ">=1.0", "^1.x"] {
            assert!(ApiRequirement::parse(bad).is_err(), "{bad}");
        }
    }
}
