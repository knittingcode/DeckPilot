//! Guest SDK for DeckPilot plugin ABI 1.
//!
//! A guest is a `cdylib` built for `wasm32-unknown-unknown` (Rust std there
//! has no file system, network, environment or process support; every such
//! call fails inside the guest). The SDK provides the required exports
//! through [`export_plugin!`] and a [`Context`] for capability calls:
//!
//! ```ignore
//! use deckpilot_plugin_guest::{ActionOutcome, Context, Params, export_plugin};
//!
//! fn handle(ctx: &mut Context, action: &str, params: &Params) -> ActionOutcome {
//!     ctx.log_info("hello");
//!     ActionOutcome::ok()
//! }
//! export_plugin!(handle);
//! ```
//!
//! On other targets the SDK compiles (for tests and linting) but every
//! capability call returns `unavailable`.

pub use deckpilot_plugin_abi::capability::{Header, HttpFetch, Log, LogLevel, Notify};
pub use deckpilot_plugin_abi::{
    ABI_VERSION, ActionOutcome, CallError, CallErrorCode, CallResult, CapabilityRequest,
    HttpMethod, ParamValue, Params,
};

/// Capability access for the current invocation.
#[derive(Debug)]
pub struct Context {
    calls: u32,
}

impl Context {
    /// Sends one capability request to the host and waits for the reply.
    pub fn call(&mut self, request: &CapabilityRequest) -> CallResult {
        self.calls += 1;
        let Ok(bytes) = serde_json::to_vec(request) else {
            return CallResult::error(CallErrorCode::BadRequest, "unserializable request");
        };
        let reply = __rt::host_call_bytes(&bytes);
        match reply {
            Some(r) => serde_json::from_slice(&r).unwrap_or_else(|_| {
                CallResult::error(CallErrorCode::Failed, "malformed reply from host")
            }),
            None => CallResult::error(CallErrorCode::Unavailable, "no host"),
        }
    }

    pub fn log(&mut self, level: LogLevel, message: &str) -> CallResult {
        self.call(&CapabilityRequest::Log(Log {
            level,
            message: message.to_owned(),
        }))
    }

    pub fn log_info(&mut self, message: &str) -> CallResult {
        self.log(LogLevel::Info, message)
    }

    pub fn fetch(&mut self, method: HttpMethod, url: &str) -> CallResult {
        self.call(&CapabilityRequest::HttpFetch(HttpFetch {
            method,
            url: url.to_owned(),
            headers: Vec::new(),
            body: None,
        }))
    }

    /// Capability calls made so far in this invocation.
    pub fn calls(&self) -> u32 {
        self.calls
    }
}

/// Generates the ABI 1 exports (`dp_abi_version`, `dp_alloc`, `dp_invoke`)
/// for a handler `fn(&mut Context, &str, &Params) -> ActionOutcome`.
#[macro_export]
macro_rules! export_plugin {
    ($handler:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn dp_abi_version() -> i32 {
            $crate::ABI_VERSION as i32
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn dp_alloc(len: i32) -> i32 {
            $crate::__rt::alloc(len)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn dp_invoke(ptr: i32, len: i32) -> i64 {
            // SAFETY: the host passes a buffer it obtained from dp_alloc.
            unsafe { $crate::__rt::invoke(ptr, len, $handler) }
        }
    };
}

/// Runtime support for [`export_plugin!`]. Not a stable API.
#[doc(hidden)]
pub mod __rt {
    use std::alloc::{Layout, alloc as raw_alloc};
    use std::cell::RefCell;

    use deckpilot_plugin_abi::guest::{InvokeInput, pack, unpack};
    use deckpilot_plugin_abi::{ActionOutcome, Params};

    use super::Context;

    #[cfg(target_arch = "wasm32")]
    #[link(wasm_import_module = "deckpilot_v1")]
    unsafe extern "C" {
        #[link_name = "host_call"]
        fn host_call_import(ptr: i32, len: i32) -> i64;
    }

    /// The raw import. Exposed so that test guests can misbehave on purpose.
    ///
    /// # Safety
    /// None needed from the guest's point of view: the host bounds-checks
    /// everything. On success the returned buffer was allocated with
    /// [`alloc`] and is owned by the caller.
    pub unsafe fn raw_host_call(ptr: i32, len: i32) -> i64 {
        #[cfg(target_arch = "wasm32")]
        {
            // SAFETY: an import call; the host validates its arguments.
            unsafe { host_call_import(ptr, len) }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (ptr, len);
            -1
        }
    }

    thread_local! {
        static LAST_OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    /// Allocates `len` bytes for the host (`dp_alloc`). 0 on failure.
    pub fn alloc(len: i32) -> i32 {
        let Ok(len) = usize::try_from(len) else {
            return 0;
        };
        if len == 0 {
            return 1; // dangling, never dereferenced or freed
        }
        let Ok(layout) = Layout::from_size_align(len, 1) else {
            return 0;
        };
        // SAFETY: non-zero size.
        let p = unsafe { raw_alloc(layout) };
        p as usize as i32
    }

    /// Takes ownership of a buffer obtained from [`alloc`].
    ///
    /// # Safety
    /// `ptr`/`len` must describe a live allocation from [`alloc`].
    unsafe fn take(ptr: i32, len: i32) -> Vec<u8> {
        let len = len as u32 as usize;
        if len == 0 {
            return Vec::new();
        }
        // SAFETY: allocated by `alloc` with Layout(len, 1).
        unsafe { Vec::from_raw_parts(ptr as u32 as usize as *mut u8, len, len) }
    }

    pub(crate) fn host_call_bytes(request: &[u8]) -> Option<Vec<u8>> {
        if !cfg!(target_arch = "wasm32") {
            return None;
        }
        // SAFETY: valid buffer; the host validates bounds anyway.
        let packed =
            unsafe { raw_host_call(request.as_ptr() as usize as i32, request.len() as i32) };
        let (ptr, len) = unpack(packed as u64);
        // SAFETY: the host wrote the reply into a buffer from dp_alloc.
        Some(unsafe { take(ptr as i32, len as i32) })
    }

    /// Body of `dp_invoke`.
    ///
    /// # Safety
    /// `ptr`/`len` must describe a buffer obtained from [`alloc`].
    pub unsafe fn invoke(
        ptr: i32,
        len: i32,
        handler: fn(&mut Context, &str, &Params) -> ActionOutcome,
    ) -> i64 {
        // SAFETY: forwarded contract.
        let input = unsafe { take(ptr, len) };
        let outcome = match serde_json::from_slice::<InvokeInput>(&input) {
            Ok(i) => {
                let mut ctx = Context { calls: 0 };
                handler(&mut ctx, &i.action, &i.params)
            }
            Err(_) => ActionOutcome::failed("bad-input", "invocation input did not parse"),
        };
        let out = serde_json::to_vec(&outcome).unwrap_or_default();
        LAST_OUTPUT.with(|slot| {
            let mut slot = slot.borrow_mut();
            *slot = out;
            pack(slot.as_ptr() as usize as u32, slot.len() as u32) as i64
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_fail_cleanly_off_wasm() {
        let mut ctx = Context { calls: 0 };
        let r = ctx.log_info("x");
        assert!(matches!(
            r,
            CallResult::Error(CallError {
                code: CallErrorCode::Unavailable,
                ..
            })
        ));
        assert_eq!(ctx.calls(), 1);
    }
}
