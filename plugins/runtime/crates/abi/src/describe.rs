//! Plugin descriptor: what a guest says it implements (optional export
//! `dp_describe`, ABI 1 compatible addition).
//!
//! Metadata comes from `manifest.json`, never from plugin code (design
//! §6.1). The descriptor exists so that the agent can **check** the module
//! against the manifest (same action ids, same parameter kinds) and refuse
//! a mismatch; it never learns anything from it. It carries no plugin id:
//! identity comes from the channel.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::value::{ParamValue, validate_name};

/// Most actions one descriptor lists (the manifest's `maxItems`).
pub const MAX_ACTIONS: usize = 64;
/// Most parameters one action declares (the manifest's `maxItems`).
pub const MAX_PARAMS: usize = 32;
/// Most options of one choice parameter.
pub const MAX_OPTIONS: usize = 64;
/// Longest choice option.
pub const MAX_OPTION_LEN: usize = 64;
/// Longest `sdk` and `apiLevel` string.
pub const MAX_LABEL_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginDescriptor {
    /// SDK that built the module, e.g. `deckpilot-plugin-sdk/0.1.0`.
    pub sdk: String,
    /// Plugin API level the module was built against, e.g. `1.0`.
    pub api_level: String,
    pub actions: Vec<ActionDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionDescriptor {
    pub id: String,
    #[serde(default)]
    pub params: Vec<ParamDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParamDescriptor {
    pub key: String,
    pub required: bool,
    pub kind: ParamKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ParamValue>,
}

/// Parameter kinds an SDK can declare. `text`, `integer`, `boolean` and
/// `choice` correspond to the manifest's parameter kinds; `number` and
/// `secret` (a credential slot key, never the secret) are SDK kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ParamKind {
    #[serde(rename_all = "camelCase")]
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u32>,
    },
    Integer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    Number {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    Boolean {},
    Choice {
        options: Vec<String>,
    },
    Secret {},
}

/// Why a descriptor is invalid. Names the problem, not the offending value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptorError {
    Label,
    TooManyActions,
    ActionId,
    DuplicateAction,
    TooManyParams,
    ParamKey,
    DuplicateParam,
    Options,
    Range,
}

impl fmt::Display for DescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Label => "sdk or apiLevel out of bounds",
            Self::TooManyActions => "too many actions",
            Self::ActionId => "invalid action id",
            Self::DuplicateAction => "duplicate action id",
            Self::TooManyParams => "too many parameters",
            Self::ParamKey => "invalid parameter key",
            Self::DuplicateParam => "duplicate parameter key",
            Self::Options => "invalid choice options",
            Self::Range => "invalid range",
        })
    }
}

impl std::error::Error for DescriptorError {}

impl PluginDescriptor {
    /// Checks every bound. The host applies this to `dp_describe` output.
    pub fn validate(&self) -> Result<(), DescriptorError> {
        let label_ok = |s: &str| !s.is_empty() && s.len() <= MAX_LABEL_LEN && s.is_ascii();
        if !label_ok(&self.sdk) || !label_ok(&self.api_level) {
            return Err(DescriptorError::Label);
        }
        if self.actions.len() > MAX_ACTIONS {
            return Err(DescriptorError::TooManyActions);
        }
        let mut ids = BTreeSet::new();
        for a in &self.actions {
            validate_name(&a.id).map_err(|_| DescriptorError::ActionId)?;
            if !ids.insert(a.id.as_str()) {
                return Err(DescriptorError::DuplicateAction);
            }
            if a.params.len() > MAX_PARAMS {
                return Err(DescriptorError::TooManyParams);
            }
            let mut keys = BTreeSet::new();
            for p in &a.params {
                validate_name(&p.key).map_err(|_| DescriptorError::ParamKey)?;
                if !keys.insert(p.key.as_str()) {
                    return Err(DescriptorError::DuplicateParam);
                }
                p.kind.validate()?;
            }
        }
        Ok(())
    }

    /// The action ids, for comparison with the manifest.
    pub fn action_ids(&self) -> BTreeSet<&str> {
        self.actions.iter().map(|a| a.id.as_str()).collect()
    }
}

impl ParamKind {
    fn validate(&self) -> Result<(), DescriptorError> {
        match self {
            Self::Integer {
                min: Some(lo),
                max: Some(hi),
            } if lo > hi => Err(DescriptorError::Range),
            Self::Number { min, max } => {
                let finite = min.is_none_or(f64::is_finite) && max.is_none_or(f64::is_finite);
                let ordered = match (min, max) {
                    (Some(lo), Some(hi)) => lo <= hi,
                    _ => true,
                };
                if finite && ordered {
                    Ok(())
                } else {
                    Err(DescriptorError::Range)
                }
            }
            Self::Choice { options } => {
                let unique: BTreeSet<&String> = options.iter().collect();
                let ok = !options.is_empty()
                    && options.len() <= MAX_OPTIONS
                    && unique.len() == options.len()
                    && options
                        .iter()
                        .all(|o| !o.is_empty() && o.chars().count() <= MAX_OPTION_LEN);
                if ok {
                    Ok(())
                } else {
                    Err(DescriptorError::Options)
                }
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<PluginDescriptor, serde_json::Error> {
        serde_json::from_str(json)
    }

    const GOOD: &str = r#"{"sdk":"deckpilot-plugin-sdk/0.1.0","apiLevel":"1.0","actions":[
        {"id":"greet","params":[
            {"key":"name","required":true,"kind":{"type":"text","maxLength":64}},
            {"key":"style","required":false,"kind":{"type":"choice","options":["plain","loud"]},
             "default":"plain"},
            {"key":"times","required":false,"kind":{"type":"integer","min":1,"max":5}},
            {"key":"ratio","required":false,"kind":{"type":"number","min":0.5}},
            {"key":"loud","required":false,"kind":{"type":"boolean"}},
            {"key":"token","required":false,"kind":{"type":"secret"}}]},
        {"id":"ping"}]}"#;

    #[test]
    fn descriptors_round_trip() {
        let d = parse(GOOD).unwrap();
        d.validate().unwrap();
        assert_eq!(d.action_ids(), BTreeSet::from(["greet", "ping"]));
        let back = serde_json::to_string(&d).unwrap();
        assert_eq!(parse(&back).unwrap(), d);
    }

    #[test]
    fn descriptors_are_strict() {
        for bad in [
            r#"{"sdk":"s","apiLevel":"1.0","actions":[],"pluginId":"acme.x"}"#,
            r#"{"sdk":"s","apiLevel":"1.0","actions":[{"id":"a","run":"x"}]}"#,
            r#"{"sdk":"s","apiLevel":"1.0","actions":[{"id":"a","params":[{"key":"k","required":true,"kind":{"type":"file"}}]}]}"#,
            r#"{"sdk":"s","actions":[]}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn descriptor_bounds() {
        let check = |json: &str| parse(json).unwrap().validate();
        let p = |kind: &str| {
            format!(
                r#"{{"sdk":"s","apiLevel":"1.0","actions":[{{"id":"a","params":[{{"key":"k","required":true,"kind":{kind}}}]}}]}}"#
            )
        };
        assert_eq!(
            check(r#"{"sdk":"","apiLevel":"1.0","actions":[]}"#),
            Err(DescriptorError::Label)
        );
        assert_eq!(
            check(r#"{"sdk":"s","apiLevel":"1.0","actions":[{"id":"a"},{"id":"a"}]}"#),
            Err(DescriptorError::DuplicateAction)
        );
        assert_eq!(
            check(r#"{"sdk":"s","apiLevel":"1.0","actions":[{"id":"1a"}]}"#),
            Err(DescriptorError::ActionId)
        );
        assert_eq!(
            check(&p(r#"{"type":"choice","options":[]}"#)),
            Err(DescriptorError::Options)
        );
        assert_eq!(
            check(&p(r#"{"type":"choice","options":["a","a"]}"#)),
            Err(DescriptorError::Options)
        );
        assert_eq!(
            check(&p(r#"{"type":"integer","min":3,"max":1}"#)),
            Err(DescriptorError::Range)
        );
        assert_eq!(
            check(&p(r#"{"type":"number","min":3.5,"max":1}"#)),
            Err(DescriptorError::Range)
        );
        let many: Vec<String> = (0..65).map(|i| format!(r#"{{"id":"a{i}"}}"#)).collect();
        let json = format!(
            r#"{{"sdk":"s","apiLevel":"1.0","actions":[{}]}}"#,
            many.join(",")
        );
        assert_eq!(check(&json), Err(DescriptorError::TooManyActions));
    }
}
