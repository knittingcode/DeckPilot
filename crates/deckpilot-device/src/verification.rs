//! How far DeckPilot's support of a model has been verified, per platform.
//!
//! The status is static data in the model registry ([`crate::ModelCatalog`]),
//! recorded with the date and a short note on the evidence. A simulated
//! device, an automated test or a driver registered at run time can never
//! raise a model to [`VerificationLevel::PhysicallyVerified`]: only the
//! built-in registry of this crate can create such an entry.

use std::fmt;

use serde::{Deserialize, Serialize};

/// An operating system DeckPilot runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

impl Platform {
    pub const ALL: [Self; 3] = [Self::Macos, Self::Windows, Self::Linux];

    /// The platform of this build, if it is one DeckPilot supports.
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

    pub fn label(self) -> &'static str {
        match self {
            Self::Macos => "macOS",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Evidence for a model on one platform, weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationLevel {
    /// DeckPilot does not support the model on this platform.
    Unsupported,
    /// A work-in-progress definition that may be incomplete or wrong.
    Experimental,
    /// The model is described (controls, image sizes) and layouts can be
    /// designed for it. There is no driver.
    Defined,
    /// A driver exists and builds for the platform; nothing more is claimed.
    DriverImplemented,
    /// The driver's automated tests (protocol checks, simulated devices) run
    /// on the platform. No physical device was used.
    AutomatedTests,
    /// The model was tested on a physical device on this platform.
    PhysicallyVerified,
}

/// The status of a model on one platform, with its evidence.
///
/// Fields are private: a [`VerificationLevel::PhysicallyVerified`] entry can
/// only be created by the built-in registry in this crate, so test code,
/// simulated devices and drivers registered at run time cannot claim it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformVerification {
    platform: Platform,
    level: VerificationLevel,
    /// Day the evidence was recorded, `YYYY-MM-DD`.
    evidence_date: Option<String>,
    /// What the level is based on, in one or two sentences.
    evidence: String,
}

impl PlatformVerification {
    /// An entry at `level`, which is capped at
    /// [`VerificationLevel::AutomatedTests`]: physical verification is
    /// recorded only in the built-in registry.
    pub fn new(
        platform: Platform,
        level: VerificationLevel,
        evidence_date: Option<&str>,
        evidence: &str,
    ) -> Self {
        Self {
            platform,
            level: level.min(VerificationLevel::AutomatedTests),
            evidence_date: evidence_date.map(str::to_owned),
            evidence: evidence.to_owned(),
        }
    }

    /// A physically verified entry. Crate-private on purpose.
    pub(crate) fn physically_verified(platform: Platform, date: &str, evidence: &str) -> Self {
        Self {
            platform,
            level: VerificationLevel::PhysicallyVerified,
            evidence_date: Some(date.to_owned()),
            evidence: evidence.to_owned(),
        }
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn level(&self) -> VerificationLevel {
        self.level
    }

    pub fn evidence_date(&self) -> Option<&str> {
        self.evidence_date.as_deref()
    }

    pub fn evidence(&self) -> &str {
        &self.evidence
    }
}

/// The verification status of a model on every platform.
///
/// A platform without an entry is [`VerificationLevel::Defined`]: the model
/// is in the registry, and nothing more is claimed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelVerification(Vec<PlatformVerification>);

impl ModelVerification {
    /// One entry per platform; a later entry for the same platform replaces
    /// an earlier one.
    pub fn new(entries: impl IntoIterator<Item = PlatformVerification>) -> Self {
        let mut list: Vec<PlatformVerification> = Vec::new();
        for entry in entries {
            list.retain(|e| e.platform != entry.platform);
            list.push(entry);
        }
        list.sort_by_key(|e| e.platform);
        Self(list)
    }

    pub fn entries(&self) -> &[PlatformVerification] {
        &self.0
    }

    pub fn entry(&self, platform: Platform) -> Option<&PlatformVerification> {
        self.0.iter().find(|e| e.platform == platform)
    }

    pub fn level(&self, platform: Platform) -> VerificationLevel {
        self.entry(platform)
            .map_or(VerificationLevel::Defined, PlatformVerification::level)
    }

    /// Platforms on which the model was tested on a physical device.
    pub fn physically_verified(&self) -> Vec<Platform> {
        self.0
            .iter()
            .filter(|e| e.level == VerificationLevel::PhysicallyVerified)
            .map(|e| e.platform)
            .collect()
    }

    /// Whether the model is listed as supported: physically verified on at
    /// least one platform. Every other model is experimental.
    pub fn is_supported(&self) -> bool {
        !self.physically_verified().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_constructor_cannot_claim_physical_verification() {
        let entry = PlatformVerification::new(
            Platform::Linux,
            VerificationLevel::PhysicallyVerified,
            Some("2026-11-13"),
            "simulated",
        );
        assert_eq!(entry.level(), VerificationLevel::AutomatedTests);
        let model = ModelVerification::new([entry]);
        assert!(!model.is_supported());
    }

    #[test]
    fn missing_platforms_are_defined_and_later_entries_win() {
        let model = ModelVerification::new([
            PlatformVerification::new(Platform::Windows, VerificationLevel::Unsupported, None, ""),
            PlatformVerification::new(
                Platform::Windows,
                VerificationLevel::DriverImplemented,
                None,
                "",
            ),
            PlatformVerification::physically_verified(Platform::Macos, "2026-11-13", "device"),
        ]);
        assert_eq!(model.entries().len(), 2);
        assert_eq!(model.entries()[0].platform(), Platform::Macos);
        assert_eq!(
            model.level(Platform::Windows),
            VerificationLevel::DriverImplemented
        );
        assert_eq!(model.level(Platform::Linux), VerificationLevel::Defined);
        assert_eq!(model.physically_verified(), vec![Platform::Macos]);
        assert!(model.is_supported());
    }

    #[test]
    fn serializes_in_camel_case() {
        let entry =
            PlatformVerification::physically_verified(Platform::Macos, "2026-11-13", "device");
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "platform": "macos",
                "level": "physicallyVerified",
                "evidenceDate": "2026-11-13",
                "evidence": "device",
            })
        );
        let back: PlatformVerification = serde_json::from_value(json).unwrap();
        assert_eq!(back, entry);
        assert!(VerificationLevel::AutomatedTests < VerificationLevel::PhysicallyVerified);
        assert!(VerificationLevel::Unsupported < VerificationLevel::Defined);
    }
}
