use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{DeviceCapabilities, ModelVerification};

/// Stable identifier of a device model, for example `elgato.streamdeck.mk2`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceModelId(String);

impl DeviceModelId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The vendor segment of the id: model ids are `<vendor>.<family>.<model>`,
    /// so `elgato.streamdeck.mk2` belongs to `elgato`.
    pub fn vendor(&self) -> &str {
        self.0.split('.').next().unwrap_or_default()
    }
}

impl fmt::Display for DeviceModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// USB identity used to recognise a model during discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbIdentity {
    pub vendor_id: u16,
    pub product_id: u16,
}

/// How well a model is supported, as one value for all platforms.
///
/// Kept for compatibility with clients that read it; the per-platform
/// [`ModelVerification`] in [`DeviceModel::verification`] is authoritative.
/// `Verified` means tested on a physical device on at least one platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HardwareSupport {
    /// The descriptor exists, but there is no driver.
    DescriptorOnly,
    /// A driver exists, but it has not been tested on a physical device.
    Unverified,
    /// A driver exists and has been tested on a physical device.
    Verified,
}

/// A device model: identity plus capabilities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceModel {
    pub id: DeviceModelId,
    pub vendor: String,
    pub name: String,
    pub usb: Option<UsbIdentity>,
    pub capabilities: DeviceCapabilities,
    pub support: HardwareSupport,
    /// Per-platform verification status, with evidence. Absent in data from
    /// older versions, which then reads as [`crate::VerificationLevel::Defined`].
    #[serde(default)]
    pub verification: ModelVerification,
}

impl DeviceModel {
    /// Listed as supported: physically verified on at least one platform.
    pub fn is_supported(&self) -> bool {
        self.verification.is_supported()
    }
}
