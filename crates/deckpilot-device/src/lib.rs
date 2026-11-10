//! Hardware-independent description of control-surface devices.
//!
//! This crate describes *what a device can do* (its capabilities) and *what
//! happened on a device* (its events). It does not talk to hardware. A future
//! HID backend translates raw reports into [`DeviceEvent`]s, and the rest of
//! DeckPilot only depends on the types defined here.

mod backend;
mod capabilities;
mod catalog;
mod control;
mod event;
mod model;
mod registry;

#[cfg(feature = "mock")]
pub mod mock;

pub use backend::{
    CombinedBackend, DeviceBackend, DeviceError, DeviceHandle, DiscoveredDevice, Frame,
    KeyStateTracker,
};
pub use capabilities::{
    DeviceCapabilities, EncoderSpec, ImageFormat, ImageSpec, ImageTransform, KeyGrid, TouchGesture,
    TouchSurface,
};
pub use catalog::{ModelCatalog, models};
pub use control::{ControlId, ControlInput, ControlKind, InputError};
pub use event::{DeviceEvent, DeviceId};
pub use model::{DeviceModel, DeviceModelId, HardwareSupport, UsbIdentity};
pub use registry::{ConnectionState, DeviceRegistry, DeviceStatus};
