use std::collections::BTreeMap;

use crate::{DeviceDescriptor, DeviceId, DeviceModel, DeviceModelId, DriverInfo, UsbIdentity};

/// Built-in device descriptors.
pub mod models {
    use crate::{
        DeviceCapabilities, DeviceModel, DeviceModelId, EncoderSpec, HardwareSupport, ImageFormat,
        ImageSpec, ImageTransform, KeyGrid, ModelVerification, Platform, PlatformVerification,
        TouchGesture, TouchSurface, UsbIdentity, VerificationLevel,
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

    /// Day the verification data below was last reviewed.
    const RECORDED: &str = "2026-11-13";

    /// A descriptor checked against the protocol library, without a driver,
    /// on every platform.
    fn defined_only() -> ModelVerification {
        ModelVerification::new(Platform::ALL.map(|platform| {
            PlatformVerification::new(
                platform,
                VerificationLevel::Defined,
                Some(RECORDED),
                "Descriptor checked against the elgato-streamdeck protocol constants by a \
                 unit test. No driver: layout design only. Never tested on a physical device.",
            )
        }))
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
            verification: defined_only(),
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
            verification: defined_only(),
        }
    }

    /// Elgato Stream Deck MK.2: 15 keys (5 x 3) with 72 x 72 px JPEG images.
    ///
    /// The values match the `elgato-streamdeck` protocol implementation used
    /// by the driver (checked by a test in `deckpilot-hid`). Physically
    /// verified on macOS only; on Windows and Linux the driver's automated
    /// tests run in CI, but no physical device was used.
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
            support: HardwareSupport::Verified,
            verification: ModelVerification::new([
                PlatformVerification::physically_verified(
                    Platform::Macos,
                    RECORDED,
                    "Physical Stream Deck MK.2 on macOS: guided `deckpilot-dev hardware-test` \
                     and real use. Display rendering, key press and release, brightness, \
                     profile switching, USB disconnect and reconnect, gestures and the \
                     dynamic Work Session timer face.",
                ),
                automated_only(Platform::Windows),
                automated_only(Platform::Linux),
            ]),
        }
    }

    /// The MK.2 driver on a platform where it was not tested on hardware.
    fn automated_only(platform: Platform) -> PlatformVerification {
        PlatformVerification::new(
            platform,
            VerificationLevel::AutomatedTests,
            Some(RECORDED),
            "The driver builds and its automated tests run in CI on this platform \
             (descriptor checked against the protocol library, simulated devices). \
             Not tested on a physical device.",
        )
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

    /// The generic descriptor of `device`, provided by `driver`; `None` for a
    /// model the catalog does not know.
    pub fn describe(
        &self,
        device: &DeviceId,
        driver: Option<DriverInfo>,
    ) -> Option<DeviceDescriptor> {
        self.get(&device.model)
            .map(|model| DeviceDescriptor::new(model, &device.serial, driver))
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
        assert_eq!(model.support, HardwareSupport::Verified);
    }

    #[test]
    fn only_the_mk2_on_macos_is_physically_verified() {
        use crate::{Platform, VerificationLevel};

        let catalog = ModelCatalog::builtin();
        let supported: Vec<_> = catalog
            .iter()
            .filter(|m| m.is_supported())
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(supported, vec![models::STREAM_DECK_MK2_ID]);

        let mk2 = models::stream_deck_mk2();
        assert_eq!(
            mk2.verification.physically_verified(),
            vec![Platform::Macos]
        );
        let macos = mk2.verification.entry(Platform::Macos).unwrap();
        assert_eq!(macos.evidence_date(), Some("2026-11-13"));
        assert!(macos.evidence().contains("hardware-test"));
        for platform in [Platform::Windows, Platform::Linux] {
            assert!(mk2.verification.level(platform) <= VerificationLevel::AutomatedTests);
        }
        for model in [models::stream_deck_xl(), models::stream_deck_plus()] {
            for platform in Platform::ALL {
                assert_eq!(
                    model.verification.level(platform),
                    VerificationLevel::Defined
                );
            }
            assert!(!model.is_supported());
        }
        // Every entry carries evidence.
        for model in catalog.iter() {
            assert_eq!(model.verification.entries().len(), 3, "{}", model.id);
            for entry in model.verification.entries() {
                assert!(entry.evidence_date().is_some() && !entry.evidence().is_empty());
            }
        }
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
