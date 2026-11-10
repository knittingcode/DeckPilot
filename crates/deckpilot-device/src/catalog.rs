use std::collections::BTreeMap;

use crate::{DeviceModel, DeviceModelId, UsbIdentity};

/// Built-in device descriptors.
pub mod models {
    use crate::{
        DeviceCapabilities, DeviceModel, DeviceModelId, HardwareSupport, ImageFormat, ImageSpec,
        ImageTransform, KeyGrid, UsbIdentity,
    };

    pub const STREAM_DECK_MK2_ID: &str = "elgato.streamdeck.mk2";

    /// Elgato Stream Deck MK.2: 15 keys (5 x 3) with 72 x 72 px JPEG images.
    ///
    /// The values match the `elgato-streamdeck` protocol implementation used
    /// by the driver (checked by a test in `deckpilot-hid`). The driver has
    /// not been tested on a physical device yet, so the support level is
    /// [`HardwareSupport::Unverified`].
    pub fn stream_deck_mk2() -> DeviceModel {
        DeviceModel {
            id: DeviceModelId::new(STREAM_DECK_MK2_ID),
            vendor: "Elgato".into(),
            name: "Stream Deck MK.2".into(),
            usb: Some(UsbIdentity {
                vendor_id: 0x0fd9,
                product_id: 0x0080,
            }),
            capabilities: DeviceCapabilities {
                keys: Some(KeyGrid {
                    rows: 3,
                    columns: 5,
                    image: Some(ImageSpec {
                        width: 72,
                        height: 72,
                        format: ImageFormat::Jpeg,
                        transform: ImageTransform {
                            flip_horizontal: true,
                            flip_vertical: true,
                            rotation: 0,
                        },
                    }),
                }),
                encoders: None,
                touch: None,
            },
            support: HardwareSupport::Unverified,
        }
    }
}

/// Collection of known device models, indexed by id.
#[derive(Debug, Clone, Default)]
pub struct ModelCatalog {
    models: BTreeMap<DeviceModelId, DeviceModel>,
}

impl ModelCatalog {
    /// Catalog containing every built-in descriptor.
    pub fn builtin() -> Self {
        let mut catalog = Self::default();
        catalog.register(models::stream_deck_mk2());
        catalog
    }

    /// Adds or replaces a model.
    pub fn register(&mut self, model: DeviceModel) {
        self.models.insert(model.id.clone(), model);
    }

    pub fn get(&self, id: &DeviceModelId) -> Option<&DeviceModel> {
        self.models.get(id)
    }

    pub fn find_by_usb(&self, usb: UsbIdentity) -> Option<&DeviceModel> {
        self.models.values().find(|model| model.usb == Some(usb))
    }

    pub fn iter(&self) -> impl Iterator<Item = &DeviceModel> {
        self.models.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ControlId, ControlInput, HardwareSupport};

    #[test]
    fn mk2_descriptor_has_fifteen_display_keys_and_nothing_else() {
        let model = models::stream_deck_mk2();
        let caps = &model.capabilities;
        assert_eq!(caps.controls().len(), 15);
        assert!(caps.encoders.is_none() && caps.touch.is_none());
        assert!(caps.has_display(ControlId::Key { index: 14 }));
        assert!(!caps.has_control(ControlId::Key { index: 15 }));
        assert!(
            caps.validate_input(ControlId::Encoder { index: 0 }, ControlInput::Press)
                .is_err()
        );
        assert_eq!(model.support, HardwareSupport::Unverified);
    }

    #[test]
    fn catalog_lookup_by_id_and_usb() {
        let catalog = ModelCatalog::builtin();
        let id = DeviceModelId::new(models::STREAM_DECK_MK2_ID);
        assert!(catalog.get(&id).is_some());
        let found = catalog.find_by_usb(UsbIdentity {
            vendor_id: 0x0fd9,
            product_id: 0x0080,
        });
        assert_eq!(found.map(|m| &m.id), Some(&id));
        assert!(
            catalog
                .find_by_usb(UsbIdentity {
                    vendor_id: 1,
                    product_id: 2
                })
                .is_none()
        );
    }
}
