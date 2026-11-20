//! The plugin's language (plugin API 1.6): `locale.get` and
//! `locale.translate` (`docs/marketplace/sdk/localization.md`).
//!
//! `locale.get` answers the language the app shows ([`LocaleReply`]): a
//! BCP 47 tag such as `pl` or `pl-PL`, nothing else about the system.
//! `locale.translate` resolves one of the plugin's **own** keys
//! ([`Translate`]) against the locale files of its installed, verified
//! package, with the fallback chain of the agent (tag, its language, the
//! plugin's `defaultLocale`, `en`, then a label made from the key). The
//! request carries no plugin id and no namespace: identity comes from the
//! channel, so a plugin can neither read nor override another plugin's or
//! DeckPilot's translations. The answer is plain text ([`Translation`]).
//!
//! There is no locale-change export: a plugin reads the locale (or
//! translates) during each invocation, and after a change of language the
//! agent asks for its keys' state again (`dp_state`), so every text it
//! draws follows.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Longest translation key, dots included.
pub const MAX_LOCALE_KEY: usize = 128;
/// Most interpolation parameters of one request.
pub const MAX_TRANSLATE_PARAMS: usize = 16;
/// Longest parameter name.
pub const MAX_TRANSLATE_PARAM_NAME: usize = 32;
/// Longest text parameter, in characters.
pub const MAX_TRANSLATE_PARAM_VALUE: usize = 256;
/// Longest locale tag in a reply.
pub const MAX_LOCALE_TAG: usize = 16;
/// Longest translated text in a reply, in characters.
pub const MAX_TRANSLATION: usize = 1024;

/// One interpolation value: a text or a number. Nothing else (objects,
/// lists, booleans) is accepted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TranslateArg {
    Text(String),
    Number(serde_json::Number),
}

impl From<&str> for TranslateArg {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<String> for TranslateArg {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

macro_rules! numbers {
    ($($t:ty),*) => {$(
        impl From<$t> for TranslateArg {
            fn from(n: $t) -> Self {
                Self::Number(serde_json::Number::from(n))
            }
        }
    )*};
}
numbers!(i32, i64, u32, u64, usize);

/// The arguments of `locale.translate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Translate {
    /// A key of the plugin's own locale files (`settings.apiToken`).
    pub key: String,
    /// `{{name}}` values; `count` also picks the plural form.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, TranslateArg>,
}

/// Why a `locale.translate` request was refused: the member, never its
/// value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslateError(pub &'static str);

/// Whether `key` is a translation key: segments of ASCII letters, digits,
/// `_` and `-` separated by single dots, at most [`MAX_LOCALE_KEY`].
pub fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_LOCALE_KEY
        && key.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

fn is_param_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= MAX_TRANSLATE_PARAM_NAME
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl Translate {
    /// Checks the key's form and the parameters: at most
    /// [`MAX_TRANSLATE_PARAMS`], valid names, texts of at most
    /// [`MAX_TRANSLATE_PARAM_VALUE`] characters without control characters.
    pub fn validate(&self) -> Result<(), TranslateError> {
        if !is_key(&self.key) {
            return Err(TranslateError("key"));
        }
        if self.params.len() > MAX_TRANSLATE_PARAMS {
            return Err(TranslateError("params"));
        }
        for (name, value) in &self.params {
            if !is_param_name(name) {
                return Err(TranslateError("params"));
            }
            if let TranslateArg::Text(text) = value
                && (text.chars().count() > MAX_TRANSLATE_PARAM_VALUE
                    || text.chars().any(char::is_control))
            {
                return Err(TranslateError("params"));
            }
        }
        Ok(())
    }
}

/// The `ok` value of `locale.get`. Unknown fields are **ignored** (host to
/// guest only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocaleReply {
    /// The app's language as a BCP 47 tag (`en`, `pl`, `pl-PL`).
    pub locale: String,
}

/// The `ok` value of `locale.translate`. Unknown fields are **ignored**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Translation {
    /// The text, interpolated. Plain text: DeckPilot never renders it as
    /// markup, and neither should the plugin.
    pub text: String,
    /// No locale of the plugin had the key: `text` is a label made from
    /// it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_bounded() {
        let ok = Translate {
            key: "repos.count".into(),
            params: BTreeMap::from([
                ("count".to_owned(), TranslateArg::from(3)),
                ("name".to_owned(), TranslateArg::from("shop")),
            ]),
        };
        assert_eq!(ok.validate(), Ok(()));
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            serde_json::json!({"key": "repos.count", "params": {"count": 3, "name": "shop"}})
        );
        for key in ["", "a..b", "a b", "../x", "ą"] {
            let bad = Translate {
                key: key.into(),
                params: BTreeMap::new(),
            };
            assert_eq!(bad.validate(), Err(TranslateError("key")), "{key}");
        }
        let mut long = ok.clone();
        long.params.insert(
            "x".into(),
            TranslateArg::Text("y".repeat(MAX_TRANSLATE_PARAM_VALUE + 1)),
        );
        assert_eq!(long.validate(), Err(TranslateError("params")));
        let mut control = ok.clone();
        control
            .params
            .insert("x".into(), TranslateArg::from("a\u{1b}[31m"));
        assert_eq!(control.validate(), Err(TranslateError("params")));
        let mut bad_name = ok.clone();
        bad_name.params.insert("a-b".into(), TranslateArg::from(1));
        assert_eq!(bad_name.validate(), Err(TranslateError("params")));
        let mut many = ok;
        for i in 0..MAX_TRANSLATE_PARAMS {
            many.params.insert(format!("p{i}"), TranslateArg::from(1));
        }
        assert_eq!(many.validate(), Err(TranslateError("params")));
        // Only texts and numbers.
        for bad in [
            r#"{"key": "a", "params": {"x": true}}"#,
            r#"{"key": "a", "params": {"x": {"y": 1}}}"#,
            r#"{"key": "a", "params": {"x": [1]}}"#,
            r#"{"key": "a", "plugin": "other.plugin"}"#,
        ] {
            assert!(serde_json::from_str::<Translate>(bad).is_err(), "{bad}");
        }
    }
}
