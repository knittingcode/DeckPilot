use std::collections::BTreeMap;

use crate::{DeviceModel, DeviceModelId, UsbIdentity};

/// Built-in device descriptors.
pub mod models {
    use crate::{
        DeviceCapabilities, DeviceModel, DeviceModelId, EncoderSpec, HardwareSupport, ImageFormat,
        ImageSpec, ImageTransform, KeyGrid, TouchGesture, TouchSurface, UsbIdentity,
    };

    pub const STREAM_DECK_MK2_ID: &str = "elgato.streamdeck.mk2";
    pub const STREAM_DECK_XL_ID: &str = "elgato.streamdeck.xl";
    pub const STREAM_DECK_PLUS_ID: &str = "elgato.streamdeck.plus";

    fn jpeg(width: u32, height: u32, mirrored: bool) -> ImageSpec {
        ImageSpec {
            width,
            height,
            format: ImageFormat::Jpeg,
            transform: ImageTransform {
                flip_horizontal: mirrored,
                flip_vertical: mirrored,
                rotation: 0,
            },
        }
    }

    fn elgato(product_id: u16) -> UsbIdentity {
        UsbIdentity {
            vendor_id: 0x0fd9,
            product_id,
        }
    }

    /// Elgato Stream Deck XL: 32 keys (8 x 4) with 96 x 96 px JPEG images.
    ///
    /// Descriptor only: layouts can be designed, but there is no driver.
    pub fn stream_deck_xl() -> DeviceModel {
        DeviceModel {
            id: DeviceModelId::new(STREAM_DECK_XL_ID),
            vendor: "Elgato".into(),
            name: "Stream Deck XL".into(),
            usb: Some(elgato(0x006c)),
            capabilities: DeviceCapabilities {
                keys: Some(KeyGrid {
                    rows: 4,
                    columns: 8,
                    image: Some(jpeg(96, 96, true)),
                }),
                encoders: None,
                touch: None,
            },
            support: HardwareSupport::DescriptorOnly,
        }
    }

    /// Elgato Stream Deck +: 8 keys (4 x 2) with 120 x 120 px images, four
    /// pressable dials and an 800 x 100 px touch strip, modelled as four
    /// 200 x 100 px zones (one above each dial).
    ///
    /// Descriptor only: layouts can be designed, but there is no driver.
    pub fn stream_deck_plus() -> DeviceModel {
        DeviceModel {
            id: DeviceModelId::new(STREAM_DECK_PLUS_ID),
            vendor: "Elgato".into(),
            name: "Stream Deck +".into(),
            usb: Some(elgato(0x0084)),
            capabilities: DeviceCapabilities {
                keys: Some(KeyGrid {
                    rows: 2,
                    columns: 4,
                    image: Some(jpeg(120, 120, false)),
                }),
                encoders: Some(EncoderSpec {
                    count: 4,
                    pressable: true,
                }),
                touch: Some(TouchSurface {
                    zones: 4,
                    image: Some(jpeg(200, 100, false)),
                    gestures: vec![
                        TouchGesture::Tap,
                        TouchGesture::LongPress,
                        TouchGesture::Swipe,
                    ],
                }),
            },
            support: HardwareSupport::DescriptorOnly,
        }
    }

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
        catalog.register(models::stream_deck_xl());
        catalog.register(models::stream_deck_plus());
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
    fn xl_and_plus_descriptors() {
        let xl = models::stream_deck_xl();
        assert_eq!(xl.capabilities.controls().len(), 32);
        assert_eq!(xl.support, HardwareSupport::DescriptorOnly);

        let plus = models::stream_deck_plus();
        let caps = &plus.capabilities;
        // 8 keys, 4 dials, 4 touch zones.
        assert_eq!(caps.controls().len(), 16);
        assert!(caps.has_display(ControlId::Key { index: 7 }));
        assert!(caps.has_display(ControlId::TouchZone { index: 3 }));
        assert!(!caps.has_display(ControlId::Encoder { index: 0 }));
        assert!(
            caps.validate_input(
                ControlId::Encoder { index: 3 },
                ControlInput::Rotate { delta: -1 }
            )
            .is_ok()
        );
        assert!(
            caps.validate_input(ControlId::Encoder { index: 3 }, ControlInput::Press)
                .is_ok()
        );
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
