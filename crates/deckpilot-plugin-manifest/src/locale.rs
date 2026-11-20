//! Plugin localization (plugin API 1.6, `docs/marketplace/sdk/localization.md`).
//!
//! A package may carry translations of its own texts in
//! `locales/<tag>.json` (`locales/en.json`, `locales/pl.json`,
//! `locales/pl-PL.json`): i18next-style JSON objects of strings, flat or
//! nested, with `{{name}}` placeholders and plural suffixes (`_one`,
//! `_few`, `_many`, `_other`). The manifest declares them in `locales` and
//! names the language its own texts fall back to in `defaultLocale`
//! (default `en`). Every locale file is a file the manifest names, so it is
//! hashed, pinned and checked like every other package file; nothing is
//! ever downloaded on its own.
//!
//! A manifest text refers to a translation with the prefix `t:`
//! (`"name": "t:plugin.name"`). Only manifests that require plugin API 1.6
//! read the prefix; for older ones `t:…` is the text itself. Identifiers
//! (the plugin id, action ids, setting keys, capability names) are never
//! translated.
//!
//! [`Catalog`] resolves keys with a fixed fallback chain: the requested
//! tag (`pl-PL`), its language (`pl`), the plugin's `defaultLocale`, then
//! `en`, then a safe fallback (the plain text where there is one, else a
//! label made from the key's last segment, [`humanize`]). Resolving never
//! fails. Results are plain text: the host never renders them as markup.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{self, Deserializer, MapAccess, Visitor};

use crate::compat::ApiLevel;

/// The plugin API level that introduced `locales`, `defaultLocale`, `t:`
/// manifest texts and the `locale.*` capability operations.
pub const LOCALIZATION_API: ApiLevel = ApiLevel::new(1, 6);
/// The prefix of a manifest text that names a translation key.
pub const KEY_PREFIX: &str = "t:";
/// The package directory of the locale files.
pub const LOCALES_DIR: &str = "locales";
/// `defaultLocale` when the manifest does not say.
pub const DEFAULT_LOCALE: &str = "en";
/// The last language of every fallback chain.
pub const FALLBACK_LOCALE: &str = "en";
/// Most locales a package declares.
pub const MAX_LOCALES: usize = 32;
/// Largest locale file, in bytes.
pub const MAX_LOCALE_FILE_BYTES: usize = 64 * 1024;
/// Most keys (leaves) in one locale file.
pub const MAX_LOCALE_KEYS: usize = 1024;
/// Longest key, dots included, in characters.
pub const MAX_KEY_LEN: usize = 128;
/// Longest translated text, in characters.
pub const MAX_VALUE_LEN: usize = 1024;
/// Deepest nesting of a locale file's objects.
pub const MAX_DEPTH: usize = 8;
/// Longest locale tag (`zh-Hant-TW` is 10).
pub const MAX_TAG_LEN: usize = 16;
/// Most interpolation parameters of one translation request.
pub const MAX_PARAMS: usize = 16;
/// Longest parameter name.
pub const MAX_PARAM_NAME: usize = 32;
/// Longest text parameter value, in characters.
pub const MAX_PARAM_VALUE: usize = 256;
/// The plural suffixes a key may end in (`Intl.PluralRules` categories).
pub const PLURAL_SUFFIXES: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

/// A problem with a locale tag, key or file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocaleError {
    #[error(
        "`{0}` is not a locale tag: use a language (`pl`), optionally with a script (`zh-Hant`) \
         and a region (`pl-PL`)"
    )]
    InvalidTag(String),
    #[error("`{0}` is not written canonically: use `{1}`")]
    NotCanonical(String, String),
    #[error(
        "`{0}` is not a translation key: use 1-{MAX_KEY_LEN} characters of letters, digits, \
         `_` and `-` in segments separated by single dots"
    )]
    InvalidKey(String),
    #[error("the file is larger than {MAX_LOCALE_FILE_BYTES} bytes ({0} bytes)")]
    TooLarge(usize),
    #[error("not valid JSON: {0}")]
    Syntax(String),
    #[error("the file must be a JSON object of strings and objects")]
    NotAnObject,
    #[error("the file has more than {MAX_LOCALE_KEYS} keys")]
    TooManyKeys,
    #[error("`{0}` is nested deeper than {MAX_DEPTH} levels")]
    TooDeep(String),
    #[error("`{0}` is longer than {MAX_VALUE_LEN} characters")]
    ValueTooLong(String),
    #[error("`{0}` contains markup; translations are plain text (no HTML tags)")]
    Markup(String),
    #[error("`{0}` contains a control character")]
    ControlCharacter(String),
    #[error("`{0}` has a malformed placeholder: use `{{{{name}}}}`")]
    Placeholder(String),
}

// ------------------------------------------------------------------ tags

/// A locale tag in its canonical form: a language of 2-3 lowercase letters,
/// optionally a script of 4 letters (`Hant`) and a region of 2 uppercase
/// letters or 3 digits (`PL`, `419`). Nothing else (no extensions, no
/// variants, no private use) is accepted: a tag names a file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocaleTag(String);

impl LocaleTag {
    /// Parses a tag in any case, with `-` or `_` separators (`pl_pl` →
    /// `pl-PL`): what the app or the system reports.
    pub fn parse(text: &str) -> Result<Self, LocaleError> {
        let invalid = || LocaleError::InvalidTag(text.to_owned());
        if text.is_empty() || text.len() > MAX_TAG_LEN {
            return Err(invalid());
        }
        let mut parts = text.split(['-', '_']);
        let language = parts.next().ok_or_else(invalid)?;
        if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic())
        {
            return Err(invalid());
        }
        let mut tag = language.to_ascii_lowercase();
        let mut script_seen = false;
        let mut region_seen = false;
        for part in parts {
            let alpha = part.bytes().all(|b| b.is_ascii_alphabetic());
            if part.len() == 4 && alpha && !script_seen && !region_seen {
                script_seen = true;
                tag.push('-');
                tag.push_str(&part[..1].to_ascii_uppercase());
                tag.push_str(&part[1..].to_ascii_lowercase());
            } else if !region_seen
                && ((part.len() == 2 && alpha)
                    || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit())))
            {
                region_seen = true;
                tag.push('-');
                tag.push_str(&part.to_ascii_uppercase());
            } else {
                return Err(invalid());
            }
        }
        Ok(Self(tag))
    }

    /// Parses a tag that must already be canonical (manifest `locales` and
    /// `defaultLocale`, which name files).
    pub fn parse_canonical(text: &str) -> Result<Self, LocaleError> {
        let tag = Self::parse(text)?;
        if tag.0 == text {
            Ok(tag)
        } else {
            Err(LocaleError::NotCanonical(text.to_owned(), tag.0))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The language subtag (`pl` of `pl-PL`).
    pub fn language(&self) -> &str {
        self.0.split('-').next().unwrap_or(&self.0)
    }

    /// The tag without its last subtag (`zh-Hant-TW` → `zh-Hant` → `zh`).
    pub fn parent(&self) -> Option<Self> {
        self.0.rfind('-').map(|at| Self(self.0[..at].to_owned()))
    }

    /// The package file of this locale (`locales/pl-PL.json`).
    pub fn file(&self) -> String {
        format!("{LOCALES_DIR}/{}.json", self.0)
    }
}

impl fmt::Display for LocaleTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The package file of the locale `tag` as written in the manifest.
pub fn locale_file(tag: &str) -> String {
    format!("{LOCALES_DIR}/{tag}.json")
}

// ------------------------------------------------------------------ keys

/// Whether `key` is a translation key: segments of ASCII letters, digits,
/// `_` and `-`, separated by single dots, at most [`MAX_KEY_LEN`]
/// characters.
pub fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY_LEN
        && key.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

/// The key a manifest text names (`"t:plugin.name"` → `plugin.name`), or
/// `None` for a plain text. The prefix alone, or with an invalid key, is
/// returned as `Some` so validation can report it.
pub fn text_key(text: &str) -> Option<&str> {
    text.strip_prefix(KEY_PREFIX)
}

/// A readable label made from a key's last segment, for a key no locale
/// has: `settings.apiToken` → `Api token`, `actions.open_repo` → `Open
/// repo`. Never empty.
pub fn humanize(key: &str) -> String {
    let segment = key.rsplit('.').next().unwrap_or(key);
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut previous_lower = false;
    for c in segment.chars() {
        if c == '_' || c == '-' {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            previous_lower = false;
            continue;
        }
        if c.is_uppercase() && previous_lower && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        previous_lower = c.is_lowercase() || c.is_ascii_digit();
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    let text = words.join(" ");
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "?".to_owned(),
    }
}

// ------------------------------------------------------------ messages

/// The texts of one locale file, flattened (`{"a": {"b": "x"}}` → `a.b`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Messages(BTreeMap<String, String>);

/// A locale file as read: strings and objects only, duplicates refused.
enum Node {
    Text(String),
    Map(Vec<(String, Node)>),
}

impl<'de> serde::Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NodeVisitor;

        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = Node;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string or an object of strings")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Node, E> {
                Ok(Node::Text(v.to_owned()))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Node, E> {
                Ok(Node::Text(v))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
                let mut seen = BTreeSet::new();
                let mut entries = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(de::Error::custom(format!("duplicate key `{key}`")));
                    }
                    if entries.len() >= MAX_LOCALE_KEYS {
                        return Err(de::Error::custom(format!(
                            "more than {MAX_LOCALE_KEYS} keys"
                        )));
                    }
                    let value = map.next_value::<Node>()?;
                    entries.push((key, value));
                }
                Ok(Node::Map(entries))
            }
        }

        deserializer.deserialize_any(NodeVisitor)
    }
}

/// Checks one translated text: length, no control characters but line
/// breaks and tabs, no HTML-like tags, well-formed `{{name}}` placeholders.
fn check_value(key: &str, value: &str) -> Result<(), LocaleError> {
    if value.chars().count() > MAX_VALUE_LEN {
        return Err(LocaleError::ValueTooLong(key.to_owned()));
    }
    if value
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(LocaleError::ControlCharacter(key.to_owned()));
    }
    let bytes = value.as_bytes();
    for (at, byte) in bytes.iter().enumerate() {
        if *byte == b'<'
            && bytes.get(at + 1).is_some_and(|next| {
                next.is_ascii_alphabetic() || matches!(next, b'/' | b'!' | b'?')
            })
        {
            return Err(LocaleError::Markup(key.to_owned()));
        }
    }
    let mut rest = value;
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            return Err(LocaleError::Placeholder(key.to_owned()));
        };
        if rest[..open].contains("}}") || !is_param_name(after[..close].trim()) {
            return Err(LocaleError::Placeholder(key.to_owned()));
        }
        rest = &after[close + 2..];
    }
    if rest.contains("}}") {
        return Err(LocaleError::Placeholder(key.to_owned()));
    }
    Ok(())
}

/// Whether `name` can name an interpolation parameter.
pub fn is_param_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= MAX_PARAM_NAME
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl Messages {
    /// Reads and checks a locale file: at most [`MAX_LOCALE_FILE_BYTES`],
    /// a JSON object whose values are strings or objects of the same kind
    /// (at most [`MAX_DEPTH`] deep), no duplicate keys, valid key segments,
    /// at most [`MAX_LOCALE_KEYS`] texts, each checked by the rules of
    /// [`LocaleError`].
    pub fn parse(bytes: &[u8]) -> Result<Self, LocaleError> {
        if bytes.len() > MAX_LOCALE_FILE_BYTES {
            return Err(LocaleError::TooLarge(bytes.len()));
        }
        let root: Node =
            serde_json::from_slice(bytes).map_err(|e| LocaleError::Syntax(e.to_string()))?;
        let Node::Map(entries) = root else {
            return Err(LocaleError::NotAnObject);
        };
        let mut messages = BTreeMap::new();
        flatten(entries, "", 1, &mut messages)?;
        Ok(Self(messages))
    }

    /// Texts from already flattened pairs (tests, tools); each is checked.
    pub fn from_pairs<'a>(
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, LocaleError> {
        let mut messages = BTreeMap::new();
        for (key, value) in pairs {
            if !is_key(key) {
                return Err(LocaleError::InvalidKey(key.to_owned()));
            }
            check_value(key, value)?;
            messages.insert(key.to_owned(), value.to_owned());
        }
        if messages.len() > MAX_LOCALE_KEYS {
            return Err(LocaleError::TooManyKeys);
        }
        Ok(Self(messages))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `key`, or one of its plural forms, is present.
    pub fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
            || PLURAL_SUFFIXES
                .iter()
                .any(|suffix| self.0.contains_key(&format!("{key}_{suffix}")))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

fn flatten(
    entries: Vec<(String, Node)>,
    prefix: &str,
    depth: usize,
    out: &mut BTreeMap<String, String>,
) -> Result<(), LocaleError> {
    for (segment, node) in entries {
        let key = if prefix.is_empty() {
            segment
        } else {
            format!("{prefix}.{segment}")
        };
        if !is_key(&key) || key.ends_with('.') {
            return Err(LocaleError::InvalidKey(key));
        }
        match node {
            Node::Text(value) => {
                check_value(&key, &value)?;
                if out.insert(key.clone(), value).is_some() {
                    // `{"a.b": …, "a": {"b": …}}`.
                    return Err(LocaleError::Syntax(format!("duplicate key `{key}`")));
                }
                if out.len() > MAX_LOCALE_KEYS {
                    return Err(LocaleError::TooManyKeys);
                }
            }
            Node::Map(children) => {
                if depth >= MAX_DEPTH {
                    return Err(LocaleError::TooDeep(key));
                }
                flatten(children, &key, depth + 1, out)?;
            }
        }
    }
    Ok(())
}

// -------------------------------------------------------------- plurals

/// The plural category of `n` in `language` (CLDR rules for English and
/// Polish; every other language uses English's `one`/`other`).
pub fn plural_category(language: &str, n: &Arg) -> &'static str {
    let integer = match n {
        Arg::Int(i) => Some(i.unsigned_abs()),
        Arg::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => {
            // Exactly integral and well inside the range of u64.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Some(f.abs() as u64)
        }
        Arg::Float(_) | Arg::Text(_) => None,
    };
    match language {
        "pl" => match integer {
            Some(1) => "one",
            Some(i) if (2..=4).contains(&(i % 10)) && !(12..=14).contains(&(i % 100)) => "few",
            Some(_) => "many",
            None => "other",
        },
        _ => match integer {
            Some(1) => "one",
            _ => "other",
        },
    }
}

// --------------------------------------------------------------- params

/// One interpolation value: text or a number. Nothing else (no objects, no
/// markup) reaches a translation.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Text(String),
    Int(i64),
    Float(f64),
}

impl fmt::Display for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.write_str(text),
            Self::Int(i) => write!(f, "{i}"),
            Self::Float(x) => write!(f, "{x}"),
        }
    }
}

impl From<&str> for Arg {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<i64> for Arg {
    fn from(n: i64) -> Self {
        Self::Int(n)
    }
}

/// Why interpolation parameters were refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParamError {
    #[error("at most {MAX_PARAMS} parameters")]
    TooMany,
    #[error("`{0}` is not a parameter name")]
    InvalidName(String),
    #[error("the parameter `{0}` is given twice")]
    Duplicate(String),
    #[error("the parameter `{0}` is longer than {MAX_PARAM_VALUE} characters")]
    TooLong(String),
    #[error("the parameter `{0}` is not a finite number")]
    NotFinite(String),
}

/// Checks interpolation parameters: at most [`MAX_PARAMS`], valid unique
/// names, texts of at most [`MAX_PARAM_VALUE`] characters, finite numbers.
pub fn check_params(params: &[(String, Arg)]) -> Result<(), ParamError> {
    if params.len() > MAX_PARAMS {
        return Err(ParamError::TooMany);
    }
    let mut names = BTreeSet::new();
    for (name, value) in params {
        if !is_param_name(name) {
            return Err(ParamError::InvalidName(name.clone()));
        }
        if !names.insert(name.as_str()) {
            return Err(ParamError::Duplicate(name.clone()));
        }
        match value {
            Arg::Text(text) if text.chars().count() > MAX_PARAM_VALUE => {
                return Err(ParamError::TooLong(name.clone()));
            }
            Arg::Float(x) if !x.is_finite() => return Err(ParamError::NotFinite(name.clone())),
            _ => {}
        }
    }
    Ok(())
}

/// `template` with each `{{name}}` replaced by its parameter. A
/// placeholder without a parameter stays as it is written.
pub fn interpolate(template: &str, params: &[(String, Arg)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = after[..close].trim();
        match params.iter().find(|(n, _)| n == name) {
            Some((_, value)) => out.push_str(&value.to_string()),
            None => out.push_str(&rest[open..open + 2 + close + 2]),
        }
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out
}

// --------------------------------------------------------------- catalog

/// Where a resolved text came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The locale that had the key.
    Locale(LocaleTag),
    /// No locale had it: the plain text or [`humanize`] of the key.
    Fallback,
}

/// A resolved text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub text: String,
    pub source: Source,
}

impl Resolved {
    pub fn is_missing(&self) -> bool {
        self.source == Source::Fallback
    }
}

/// One plugin's verified translations, with its default locale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    default: LocaleTag,
    locales: BTreeMap<LocaleTag, Messages>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self::empty()
    }
}

impl Catalog {
    /// A catalog without translations: every key falls back.
    pub fn empty() -> Self {
        Self {
            default: LocaleTag(DEFAULT_LOCALE.to_owned()),
            locales: BTreeMap::new(),
        }
    }

    pub fn new(default: LocaleTag, locales: BTreeMap<LocaleTag, Messages>) -> Self {
        Self { default, locales }
    }

    pub fn default_locale(&self) -> &LocaleTag {
        &self.default
    }

    pub fn locales(&self) -> impl Iterator<Item = &LocaleTag> {
        self.locales.keys()
    }

    pub fn messages(&self, tag: &LocaleTag) -> Option<&Messages> {
        self.locales.get(tag)
    }

    /// The locales tried for `requested`, in order and present in the
    /// catalog: the tag, its parents (`pl-PL` → `pl`), the default locale,
    /// then `en`.
    pub fn chain(&self, requested: Option<&LocaleTag>) -> Vec<&LocaleTag> {
        let mut wanted: Vec<LocaleTag> = Vec::new();
        let mut next = requested.cloned();
        while let Some(tag) = next {
            next = tag.parent();
            wanted.push(tag);
        }
        wanted.push(self.default.clone());
        wanted.push(LocaleTag(FALLBACK_LOCALE.to_owned()));
        let mut chain: Vec<&LocaleTag> = Vec::new();
        for tag in &wanted {
            if let Some((present, _)) = self.locales.get_key_value(tag)
                && !chain.contains(&present)
            {
                chain.push(present);
            }
        }
        chain
    }

    /// The text of `key` in the first locale of the chain that has it (a
    /// plural form for the parameter `count`, chosen by that locale's
    /// language), interpolated. Without one: `fallback` (interpolated), or
    /// [`humanize`] of the key.
    pub fn translate(
        &self,
        requested: Option<&LocaleTag>,
        key: &str,
        params: &[(String, Arg)],
        fallback: Option<&str>,
    ) -> Resolved {
        let count = params.iter().find(|(n, _)| n == "count").map(|(_, v)| v);
        for tag in self.chain(requested) {
            let Some(messages) = self.locales.get(tag) else {
                continue;
            };
            let found = match count {
                Some(count) => {
                    let category = plural_category(tag.language(), count);
                    messages
                        .get(&format!("{key}_{category}"))
                        .or_else(|| messages.get(&format!("{key}_other")))
                        .or_else(|| messages.get(key))
                }
                None => messages.get(key),
            };
            if let Some(template) = found {
                return Resolved {
                    text: interpolate(template, params),
                    source: Source::Locale(tag.clone()),
                };
            }
        }
        Resolved {
            text: fallback.map_or_else(|| humanize(key), |text| interpolate(text, params)),
            source: Source::Fallback,
        }
    }

    /// A manifest text in `requested`: a `t:` key is translated (falling
    /// back to [`humanize`]); a plain text is returned as it is. Calls
    /// `missing` with a key no locale has.
    pub fn resolve_text(
        &self,
        requested: Option<&LocaleTag>,
        text: &str,
        missing: &mut dyn FnMut(&str),
    ) -> String {
        match text_key(text) {
            Some(key) if is_key(key) => {
                let resolved = self.translate(requested, key, &[], None);
                if resolved.is_missing() {
                    missing(key);
                }
                resolved.text
            }
            // Refused by validation; never shown as a raw key.
            Some(key) => humanize(key),
            None => text.to_owned(),
        }
    }

    /// Every text `key` has in any locale (to recognise a title that was
    /// produced from it in another language).
    pub fn all_texts(&self, key: &str) -> BTreeSet<&str> {
        self.locales
            .values()
            .filter_map(|messages| messages.get(key))
            .collect()
    }
}

/// Truncates `text` to `max` characters (a translation longer than the
/// field it fills).
pub fn clamp(text: &mut String, max: usize) {
    if let Some((at, _)) = text.char_indices().nth(max) {
        text.truncate(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(text: &str) -> LocaleTag {
        LocaleTag::parse(text).unwrap()
    }

    #[test]
    fn tags_are_canonical_language_script_region() {
        assert_eq!(tag("pl").as_str(), "pl");
        assert_eq!(tag("pl_pl").as_str(), "pl-PL");
        assert_eq!(tag("ZH-hant-tw").as_str(), "zh-Hant-TW");
        assert_eq!(tag("es-419").as_str(), "es-419");
        assert_eq!(tag("pl-PL").parent(), Some(tag("pl")));
        assert_eq!(tag("pl").parent(), None);
        assert_eq!(tag("pl-PL").file(), "locales/pl-PL.json");
        for bad in [
            "",
            "p",
            "polish",
            "pl-",
            "pl-PL-x",
            "en-US-u-ca",
            "x-private",
            "pl PL",
            "pl-PLL",
            "../en",
            "en/x",
            "pl-12",
        ] {
            assert!(LocaleTag::parse(bad).is_err(), "{bad}");
        }
        assert!(matches!(
            LocaleTag::parse_canonical("pl-pl"),
            Err(LocaleError::NotCanonical(_, canonical)) if canonical == "pl-PL"
        ));
        assert!(LocaleTag::parse_canonical("pl-PL").is_ok());
    }

    #[test]
    fn keys_and_humanized_labels() {
        for good in ["a", "plugin.name", "settings.api_token", "x-1.y_2"] {
            assert!(is_key(good), "{good}");
        }
        for bad in ["", ".a", "a.", "a..b", "a b", "a/b", "ą", &"a".repeat(129)] {
            assert!(!is_key(bad), "{bad}");
        }
        assert_eq!(humanize("settings.apiToken"), "Api token");
        assert_eq!(humanize("actions.open_repo"), "Open repo");
        assert_eq!(humanize("refresh-interval"), "Refresh interval");
        assert_eq!(humanize("x"), "X");
        assert_eq!(humanize("___"), "?");
    }

    #[test]
    fn files_are_strings_and_objects_flattened() {
        let m = Messages::parse(
            br#"{"plugin": {"name": "Repo", "count_one": "{{count}} repo"}, "flat.key": "x"}"#,
        )
        .unwrap();
        assert_eq!(m.get("plugin.name"), Some("Repo"));
        assert_eq!(m.get("flat.key"), Some("x"));
        assert!(m.has("plugin.count"));
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn bad_files_are_refused() {
        type Check = fn(&LocaleError) -> bool;
        let cases: &[(&[u8], Check)] = &[
            (b"[]", |e| matches!(e, LocaleError::Syntax(_))),
            (b"\"x\"", |e| matches!(e, LocaleError::NotAnObject)),
            (b"{\"a\": 1}", |e| matches!(e, LocaleError::Syntax(_))),
            (b"{\"a\": [\"x\"]}", |e| matches!(e, LocaleError::Syntax(_))),
            (b"{\"a\": null}", |e| matches!(e, LocaleError::Syntax(_))),
            (
                b"{\"a\": \"x\", \"a\": \"y\"}",
                |e| matches!(e, LocaleError::Syntax(m) if m.contains("duplicate")),
            ),
            (
                b"{\"a.b\": \"x\", \"a\": {\"b\": \"y\"}}",
                |e| matches!(e, LocaleError::Syntax(m) if m.contains("duplicate")),
            ),
            (b"{\"a b\": \"x\"}", |e| {
                matches!(e, LocaleError::InvalidKey(_))
            }),
            (b"{\"a\": \"<b>bold</b>\"}", |e| {
                matches!(e, LocaleError::Markup(_))
            }),
            (b"{\"a\": \"<img src=x onerror=1>\"}", |e| {
                matches!(e, LocaleError::Markup(_))
            }),
            (b"{\"a\": \"<!-- x -->\"}", |e| {
                matches!(e, LocaleError::Markup(_))
            }),
            (b"{\"a\": \"bell \\u0007\"}", |e| {
                matches!(e, LocaleError::ControlCharacter(_))
            }),
            (b"{\"a\": \"{{count\"}", |e| {
                matches!(e, LocaleError::Placeholder(_))
            }),
            (b"{\"a\": \"{{val, number}}\"}", |e| {
                matches!(e, LocaleError::Placeholder(_))
            }),
            (b"{\"a\": \"x }}\"}", |e| {
                matches!(e, LocaleError::Placeholder(_))
            }),
        ];
        for (bytes, expected) in cases {
            let error = Messages::parse(bytes).unwrap_err();
            assert!(
                expected(&error),
                "{}: {error}",
                String::from_utf8_lossy(bytes)
            );
        }
        // Comparisons and line breaks are fine.
        assert!(Messages::parse(b"{\"a\": \"1 < 2 > 0\\nnext\\tline\"}").is_ok());

        let deep = format!(
            "{}\"x\"{}",
            "{\"a\": ".repeat(MAX_DEPTH + 1),
            "}".repeat(MAX_DEPTH + 1)
        );
        assert!(matches!(
            Messages::parse(deep.as_bytes()),
            Err(LocaleError::TooDeep(_))
        ));
        let long = format!("{{\"a\": \"{}\"}}", "x".repeat(MAX_VALUE_LEN + 1));
        assert!(matches!(
            Messages::parse(long.as_bytes()),
            Err(LocaleError::ValueTooLong(_))
        ));
        let big = format!("{{\"a\": \"{}\"}}", " ".repeat(MAX_LOCALE_FILE_BYTES));
        assert!(matches!(
            Messages::parse(big.as_bytes()),
            Err(LocaleError::TooLarge(_))
        ));
        let many: String = (0..=MAX_LOCALE_KEYS)
            .map(|i| format!("\"k{i}\": \"v\""))
            .collect::<Vec<_>>()
            .join(",");
        assert!(Messages::parse(format!("{{{many}}}").as_bytes()).is_err());
    }

    fn catalog() -> Catalog {
        let mut locales = BTreeMap::new();
        locales.insert(
            tag("en"),
            Messages::from_pairs([
                ("name", "Repositories"),
                ("only.en", "English only"),
                ("repos_one", "{{count}} repository"),
                ("repos_other", "{{count}} repositories"),
                ("hello", "Hello, {{name}}!"),
            ])
            .unwrap(),
        );
        locales.insert(
            tag("pl"),
            Messages::from_pairs([
                ("name", "Repozytoria"),
                ("repos_one", "{{count}} repozytorium"),
                ("repos_few", "{{count}} repozytoria"),
                ("repos_many", "{{count}} repozytoriów"),
                ("repos_other", "{{count}} repozytorium"),
                ("hello", "Cześć, {{name}}!"),
            ])
            .unwrap(),
        );
        locales.insert(
            tag("pl-PL"),
            Messages::from_pairs([("name", "Repozytoria (PL)")]).unwrap(),
        );
        locales.insert(
            tag("de"),
            Messages::from_pairs([("name", "Repositorien"), ("only.de", "Nur Deutsch")]).unwrap(),
        );
        Catalog::new(tag("de"), locales)
    }

    fn text(c: &Catalog, requested: &str, key: &str) -> Resolved {
        c.translate(Some(&tag(requested)), key, &[], None)
    }

    #[test]
    fn the_fallback_chain_is_tag_language_default_english_humanized() {
        let c = catalog();
        let chain: Vec<&str> = c
            .chain(Some(&tag("pl-PL")))
            .into_iter()
            .map(LocaleTag::as_str)
            .collect();
        assert_eq!(chain, ["pl-PL", "pl", "de", "en"]);
        // pl-PL itself.
        assert_eq!(text(&c, "pl-PL", "name").text, "Repozytoria (PL)");
        assert_eq!(
            text(&c, "pl-PL", "name").source,
            Source::Locale(tag("pl-PL"))
        );
        // pl-PL lacks it: pl.
        assert_eq!(text(&c, "pl-PL", "hello").source, Source::Locale(tag("pl")));
        assert_eq!(text(&c, "pl", "name").text, "Repozytoria");
        // Neither: the default locale, then English.
        assert_eq!(text(&c, "pl-PL", "only.de").text, "Nur Deutsch");
        assert_eq!(text(&c, "pl-PL", "only.en").text, "English only");
        // An unknown language: default first.
        assert_eq!(text(&c, "fr-FR", "name").text, "Repositorien");
        // Nobody: humanized, or the plain text given.
        let missing = text(&c, "pl", "settings.apiToken");
        assert!(missing.is_missing());
        assert_eq!(missing.text, "Api token");
        assert_eq!(
            c.translate(Some(&tag("pl")), "x.y", &[], Some("Plain"))
                .text,
            "Plain"
        );
        // No locale at all, no request.
        assert_eq!(
            Catalog::empty().translate(None, "a.bC", &[], None).text,
            "B c"
        );
        assert_eq!(c.translate(None, "name", &[], None).text, "Repositorien");
    }

    #[test]
    fn plurals_follow_each_languages_rules() {
        let c = catalog();
        let count = |requested: &str, n: Arg| {
            c.translate(
                Some(&tag(requested)),
                "repos",
                &[("count".to_owned(), n)],
                None,
            )
            .text
        };
        assert_eq!(count("en", Arg::Int(1)), "1 repository");
        assert_eq!(count("en", Arg::Int(2)), "2 repositories");
        assert_eq!(count("pl", Arg::Int(1)), "1 repozytorium");
        for n in [2, 3, 4, 22, 23, 104] {
            assert_eq!(count("pl", Arg::Int(n)), format!("{n} repozytoria"), "{n}");
        }
        for n in [0, 5, 11, 12, 13, 14, 21, 25, 112] {
            assert_eq!(count("pl", Arg::Int(n)), format!("{n} repozytoriów"), "{n}");
        }
        assert_eq!(count("pl", Arg::Float(1.5)), "1.5 repozytorium");
        assert_eq!(count("pl", Arg::Float(2.0)), "2 repozytoria");
        // Polish rules even through pl-PL, which has no such key.
        assert_eq!(count("pl-PL", Arg::Int(5)), "5 repozytoriów");
        assert_eq!(plural_category("pl", &Arg::Int(-2)), "few");
        assert_eq!(plural_category("cs", &Arg::Int(3)), "other");
    }

    #[test]
    fn interpolation_is_plain_and_bounded() {
        let c = catalog();
        let hello = c.translate(
            Some(&tag("pl")),
            "hello",
            &[("name".to_owned(), Arg::from("<b>Ala</b>"))],
            None,
        );
        // Plain text: whatever the value holds stays text.
        assert_eq!(hello.text, "Cześć, <b>Ala</b>!");
        assert_eq!(interpolate("{{ a }} {{b}}", &[]), "{{ a }} {{b}}");
        assert_eq!(
            interpolate("{{ a }}+{{a}}", &[("a".into(), Arg::Int(7))]),
            "7+7"
        );
        assert_eq!(interpolate("open {{", &[]), "open {{");

        assert_eq!(check_params(&[("count".into(), Arg::Int(3))]), Ok(()));
        let too_many: Vec<(String, Arg)> = (0..=MAX_PARAMS)
            .map(|i| (format!("p{i}"), Arg::Int(1)))
            .collect();
        assert_eq!(check_params(&too_many), Err(ParamError::TooMany));
        assert!(matches!(
            check_params(&[("a b".into(), Arg::Int(1))]),
            Err(ParamError::InvalidName(_))
        ));
        assert!(matches!(
            check_params(&[("a".into(), Arg::Int(1)), ("a".into(), Arg::Int(2))]),
            Err(ParamError::Duplicate(_))
        ));
        assert!(matches!(
            check_params(&[("a".into(), Arg::Text("x".repeat(MAX_PARAM_VALUE + 1)))]),
            Err(ParamError::TooLong(_))
        ));
        assert!(matches!(
            check_params(&[("a".into(), Arg::Float(f64::NAN))]),
            Err(ParamError::NotFinite(_))
        ));
    }

    #[test]
    fn manifest_texts_resolve_or_stay_plain() {
        let c = catalog();
        let mut missing = Vec::new();
        let pl = tag("pl");
        let mut note = |key: &str| missing.push(key.to_owned());
        assert_eq!(
            c.resolve_text(Some(&pl), "t:name", &mut note),
            "Repozytoria"
        );
        assert_eq!(
            c.resolve_text(Some(&pl), "Plain name", &mut note),
            "Plain name"
        );
        assert_eq!(
            c.resolve_text(Some(&pl), "t:no.suchKey", &mut note),
            "Such key"
        );
        assert_eq!(c.resolve_text(Some(&pl), "t:bad key", &mut note), "Bad key");
        assert_eq!(missing, ["no.suchKey"]);
        assert_eq!(
            c.all_texts("name"),
            BTreeSet::from([
                "Repositories",
                "Repozytoria",
                "Repozytoria (PL)",
                "Repositorien"
            ])
        );
        let mut long = "ąę".repeat(10);
        clamp(&mut long, 3);
        assert_eq!(long, "ąęą");
    }
}
