//! Vendor-neutral description of one physical (or simulated) device.

use serde::{Deserialize, Serialize};

use crate::{DeviceCapabilities, DeviceId, DeviceModel, DeviceModelId, ModelVerification};

/// The driver that provides a device: its stable identifier (for example
/// `elgato-hid` or `mock`) and the version of its implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverInfo {
    pub id: String,
    pub version: String,
}

/// Everything DeckPilot knows about a device, without vendor-specific types.
///
/// This is the stable device context for code outside the drivers (variable
/// resolution, the configuration app): identity, capabilities, the driver and
/// the verification status of the model. Fields are only ever added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDescriptor {
    /// Vendor identifier, the first segment of the model id, e.g. `elgato`.
    pub vendor_id: String,
    /// Vendor name for display, e.g. `Elgato`.
    pub vendor_name: String,
    /// Stable model identifier, e.g. `elgato.streamdeck.mk2`.
    pub model_id: DeviceModelId,
    /// Model name for display, e.g. `Stream Deck MK.2`.
    pub display_name: String,
    /// Hardware identity: the serial number reported by the device (or the
    /// simulated serial of a simulated device).
    pub serial: String,
    pub capabilities: DeviceCapabilities,
    /// The driver that provides the device; `None` when it is not known,
    /// e.g. for a device that is not connected.
    pub driver: Option<DriverInfo>,
    /// Per-platform verification status of the model.
    pub verification: ModelVerification,
}

impl DeviceDescriptor {
    /// The descriptor of `device`, a device of `model`.
    pub fn new(model: &DeviceModel, serial: &str, driver: Option<DriverInfo>) -> Self {
        Self {
            vendor_id: model.id.vendor().to_owned(),
            vendor_name: model.vendor.clone(),
            model_id: model.id.clone(),
            display_name: model.name.clone(),
            serial: serial.to_owned(),
            capabilities: model.capabilities.clone(),
            driver,
            verification: model.verification.clone(),
        }
    }

    /// The identifier used for the device everywhere else (model + serial).
    pub fn device_id(&self) -> DeviceId {
        DeviceId {
            model: self.model_id.clone(),
            serial: self.serial.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelCatalog, Platform, VerificationLevel, models};

    #[test]
    fn describes_a_known_device() {
        let catalog = ModelCatalog::builtin();
        let device = DeviceId {
            model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
            serial: "A1".into(),
        };
        let driver = DriverInfo {
            id: "elgato-hid".into(),
            version: "1.2.3".into(),
        };
        let descriptor = catalog.describe(&device, Some(driver.clone())).unwrap();
        assert_eq!(descriptor.vendor_id, "elgato");
        assert_eq!(descriptor.vendor_name, "Elgato");
        assert_eq!(descriptor.display_name, "Stream Deck MK.2");
        assert_eq!(descriptor.serial, "A1");
        assert_eq!(descriptor.driver, Some(driver));
        assert_eq!(descriptor.capabilities.controls().len(), 15);
        assert_eq!(
            descriptor.verification.level(Platform::Macos),
            VerificationLevel::PhysicallyVerified
        );
        assert_eq!(descriptor.device_id(), device);

        let json = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(json["vendorId"], "elgato");
        assert_eq!(json["modelId"], "elgato.streamdeck.mk2");
        assert_eq!(json["driver"]["id"], "elgato-hid");
        assert_eq!(json["verification"][0]["platform"], "macos");
        let back: DeviceDescriptor = serde_json::from_value(json).unwrap();
        assert_eq!(back, descriptor);

        let unknown = DeviceId {
            model: DeviceModelId::new("acme.unknown"),
            serial: "X".into(),
        };
        assert!(catalog.describe(&unknown, None).is_none());
    }
}
