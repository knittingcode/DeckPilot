//! Backend commands contributed by feature modules. Each submodule belongs to
//! one feature module and checks that the module is enabled.

// Tauri injects `State` and deserialized arguments by value.
#![allow(clippy::needless_pass_by_value)]

pub mod projects;
