//! Renders [`KeyFace`](deckpilot_core::KeyFace)s into device-independent
//! frames: state marks, appearance and icons. The renderer lives in
//! `deckpilot-render` so the configuration app previews keys exactly as
//! the agent draws them.

pub use deckpilot_render::{FrameCache, Icons, render, render_with};
