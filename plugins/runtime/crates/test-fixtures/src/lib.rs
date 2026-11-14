//! Test fixtures: the test guest module, built from `guest/test-guest` by
//! this crate's build script.
#![forbid(unsafe_code)]

/// `guest/test-guest` compiled for wasm32-unknown-unknown.
pub static TEST_GUEST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/test_guest.wasm"));

#[cfg(test)]
mod tests {
    #[test]
    fn guest_is_a_wasm_module() {
        assert_eq!(&super::TEST_GUEST[..4], b"\0asm");
    }
}
