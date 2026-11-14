//! Spike guest: stands in for the QuickJS engine module. Every export is a
//! deliberate misbehaviour or a probe. The only import is
//! `deckpilot.host_call`; Rust std on wasm32-unknown-unknown has no
//! file system, network, environment or clock implementation.

use core::hint::black_box;

#[link(wasm_import_module = "deckpilot")]
unsafe extern "C" {
    /// Sends `len` bytes of request JSON at `ptr` to the host. The host
    /// writes the reply into memory obtained from `guest_alloc` and returns
    /// `(reply_ptr << 32) | reply_len`.
    fn host_call(ptr: *const u8, len: usize) -> u64;
}

#[unsafe(no_mangle)]
pub extern "C" fn guest_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let p = v.as_mut_ptr();
    core::mem::forget(v);
    p
}

/// # Safety
/// `ptr`/`len` must come from `guest_alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guest_free(ptr: *mut u8, len: usize) {
    unsafe { drop(Vec::from_raw_parts(ptr, 0, len)) }
}

#[unsafe(no_mangle)]
pub extern "C" fn add(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}

/// Infinite loop with no calls out of the guest.
#[unsafe(no_mangle)]
pub extern "C" fn spin() -> i32 {
    let mut x = 0u64;
    loop {
        x = black_box(x.wrapping_add(1));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn trap_unreachable() {
    core::arch::wasm32::unreachable()
}

#[unsafe(no_mangle)]
pub extern "C" fn trap_panic() {
    panic!("guest panic")
}

/// Reads far outside linear memory.
#[unsafe(no_mangle)]
pub extern "C" fn trap_oob() -> i32 {
    unsafe { core::ptr::read_volatile(black_box(0xFFFF_FFF0usize) as *const i32) }
}

#[unsafe(no_mangle)]
#[allow(unconditional_recursion)]
pub extern "C" fn trap_stack_overflow(n: u32) -> u32 {
    let buf = black_box([n; 64]);
    black_box(trap_stack_overflow(black_box(n.wrapping_add(1)))).wrapping_add(buf[3])
}

/// Allocates 1 MiB chunks until the allocator fails (which aborts, i.e. traps).
#[unsafe(no_mangle)]
pub extern "C" fn memory_bomb() -> u32 {
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    loop {
        chunks.push(black_box(vec![1u8; 1 << 20]));
    }
}

/// Grows linear memory with `memory.grow` until it fails; returns the final
/// size in 64 KiB pages. With StoreLimits the grow returns -1, no trap.
#[unsafe(no_mangle)]
pub extern "C" fn grow_raw() -> u32 {
    while core::arch::wasm32::memory_grow(0, 16) != usize::MAX {}
    core::arch::wasm32::memory_size(0) as u32
}

// Ambient authority probes through Rust std. Each returns 1 if the
// operation succeeded, 0 if std reported an error. Some std functions on
// wasm32-unknown-unknown panic instead (a trap); that is also a denial.

#[unsafe(no_mangle)]
pub extern "C" fn probe_fs_read() -> u32 {
    std::fs::read("/etc/passwd").is_ok() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn probe_fs_write() -> u32 {
    std::fs::write("/tmp/deckpilot-spike-guest", b"x").is_ok() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn probe_read_dir() -> u32 {
    std::fs::read_dir("/").is_ok() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn probe_env() -> u32 {
    (std::env::var("HOME").is_ok() || std::env::vars().next().is_some()) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn probe_args() -> u32 {
    std::env::args().next().is_some() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn probe_tcp() -> u32 {
    std::net::TcpStream::connect("127.0.0.1:80").is_ok() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn probe_clock() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() > 0)
        .unwrap_or(false) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn probe_spawn() -> u32 {
    std::process::Command::new("/bin/sh").output().is_ok() as u32
}

/// Forwards one request (JSON bytes) to the host and returns the packed reply.
#[unsafe(no_mangle)]
pub extern "C" fn relay(ptr: *const u8, len: usize) -> u64 {
    unsafe { host_call(ptr, len) }
}

/// Sends the same request `n` times (for latency measurement); frees each
/// reply and returns the length of the last one.
#[unsafe(no_mangle)]
pub extern "C" fn relay_n(ptr: *const u8, len: usize, n: u32) -> u32 {
    let mut last = 0;
    for _ in 0..n {
        let packed = unsafe { host_call(ptr, len) };
        let (p, l) = ((packed >> 32) as usize, (packed & 0xFFFF_FFFF) as usize);
        last = l as u32;
        unsafe { guest_free(p as *mut u8, l) };
    }
    last
}
