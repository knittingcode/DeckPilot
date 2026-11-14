//! Size limits of ABI 1. Both sides enforce them; the receiving side never
//! trusts the sender to have done so.

/// Largest frame the supervisor sends to the host (design §3.6: 2 MiB).
pub const MAX_FRAME_TO_HOST: usize = 2 << 20;
/// Largest frame the host sends to the supervisor (design §3.6: 256 KiB).
pub const MAX_FRAME_TO_SUPERVISOR: usize = 256 << 10;
/// Largest `load` frame (the module bytes, base64 in JSON). Only the first
/// frame after `ready` may be this large.
pub const MAX_LOAD_FRAME: usize = 24 << 20;
/// Largest wasm module the host accepts.
pub const MAX_MODULE_BYTES: usize = 16 << 20;

/// Largest invocation input (JSON) the host writes into guest memory.
pub const MAX_GUEST_INPUT: usize = 1 << 20;
/// Largest invocation output (JSON) the host reads from guest memory. Leaves
/// room for the frame envelope inside [`MAX_FRAME_TO_SUPERVISOR`].
pub const MAX_GUEST_OUTPUT: usize = 192 << 10;
/// Largest `host_call` request a guest may pass.
pub const MAX_HOST_CALL_REQUEST: usize = 64 << 10;
/// Largest `host_call` reply written back into guest memory.
pub const MAX_HOST_CALL_REPLY: usize = 1 << 20;
/// `host_call`s one invocation may make. More is a guest fault (trap).
pub const MAX_HOST_CALLS_PER_INVOCATION: u32 = 256;

/// Action names and parameter keys.
pub const MAX_NAME_LEN: usize = 64;
/// Strings inside parameters and result data.
pub const MAX_STRING_LEN: usize = 16 << 10;
/// Elements of one list value.
pub const MAX_LIST_LEN: usize = 256;
/// Entries of a parameter or data map.
pub const MAX_MAP_ENTRIES: usize = 64;
/// Nesting of list values.
pub const MAX_VALUE_DEPTH: usize = 4;
/// Result messages.
pub const MAX_MESSAGE_LEN: usize = 1024;
/// Error codes (`[a-z][a-z0-9-]*`).
pub const MAX_CODE_LEN: usize = 64;

/// URL in `http.fetch`.
pub const MAX_URL_LEN: usize = 8 << 10;
/// Headers in one `http.fetch`.
pub const MAX_HEADERS: usize = 32;
/// One header name or value.
pub const MAX_HEADER_LEN: usize = 4 << 10;
/// Request body in one `http.fetch`.
pub const MAX_BODY_LEN: usize = 32 << 10;
/// One log line (design §5.2).
pub const MAX_LOG_LEN: usize = 2 << 10;
/// Notification title and body (design §5.2).
pub const MAX_NOTIFY_TITLE: usize = 64;
/// Notification body (design §5.2).
pub const MAX_NOTIFY_BODY: usize = 300;
/// Storage key (design §5.2).
pub const MAX_STORAGE_KEY: usize = 128;
/// Storage value, serialized (design §5.2).
pub const MAX_STORAGE_VALUE: usize = 64 << 10;
