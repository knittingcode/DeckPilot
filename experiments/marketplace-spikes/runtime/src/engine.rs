//! wasmtime host for the spike guest.
//!
//! The linker defines exactly one import, `deckpilot.host_call`. No WASI is
//! linked, so a module importing anything else fails to instantiate.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use wasmtime::{
    AsContextMut, Caller, Config, Engine, Instance, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder,
};

pub use wasmtime::{Error, Result, Trap};

/// The guest module, built by build.rs.
pub static GUEST_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/guest.wasm"));

/// Largest request a guest may pass to `host_call`.
pub const MAX_REQUEST: usize = 256 * 1024;

/// Epoch tick used by [`EpochTicker`].
pub const TICK: Duration = Duration::from_millis(10);

#[derive(Clone, Debug)]
pub struct Limits {
    /// Fuel per Store; `None` disables fuel accounting for this Store.
    pub fuel: Option<u64>,
    /// Epoch ticks the guest may run between host yields; `None` = no deadline.
    pub epoch_ticks: Option<u64>,
    /// Linear memory cap in bytes.
    pub memory_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        // Values from the design (§3.6): 250 ms deadline, 64 MiB memory.
        Self {
            fuel: None,
            epoch_ticks: Some(25),
            memory_bytes: 64 << 20,
        }
    }
}

/// Handles one `host_call` request (raw bytes from guest memory) and
/// returns the reply bytes.
pub type Handler = Box<dyn FnMut(&[u8]) -> Vec<u8>>;

pub struct HostState {
    limits: StoreLimits,
    handler: Handler,
    epoch_ticks: Option<u64>,
    pub host_calls: u64,
}

/// One compiled engine and module plus a linker with only our import.
pub struct Runtime {
    pub engine: Engine,
    pub module: Module,
    linker: Linker<HostState>,
}

pub fn engine() -> Result<Engine> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    // Keep native stack use bounded so deep guest recursion traps early.
    config.max_wasm_stack(512 * 1024);
    Engine::new(&config)
}

impl Runtime {
    pub fn new(wasm: &[u8]) -> Result<Self> {
        let engine = engine()?;
        let module = Module::new(&engine, wasm)?;
        Self::with_module(engine, module)
    }

    pub fn with_module(engine: Engine, module: Module) -> Result<Self> {
        let mut linker = Linker::new(&engine);
        linker.func_wrap("deckpilot", "host_call", host_call)?;
        Ok(Self {
            engine,
            module,
            linker,
        })
    }

    /// A fresh Store and instance. A Store is never reused after a trap.
    pub fn instantiate(&self, limits: &Limits, handler: Handler) -> Result<Plugin> {
        let state = HostState {
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                .instances(1)
                .memories(1)
                .tables(1)
                .build(),
            handler,
            epoch_ticks: limits.epoch_ticks,
            host_calls: 0,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(limits.fuel.unwrap_or(u64::MAX))?;
        store.set_epoch_deadline(limits.epoch_ticks.unwrap_or(u64::MAX / 2));
        let instance = self.linker.instantiate(&mut store, &self.module)?;
        Ok(Plugin { store, instance })
    }
}

fn host_call(mut caller: Caller<'_, HostState>, ptr: u32, len: u32) -> Result<u64> {
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| Error::msg("guest has no memory export"))?;
    let (ptr, len) = (ptr as usize, len as usize);
    if len > MAX_REQUEST {
        return Err(Error::msg("host_call request too large"));
    }
    // Bounds-checked copy out of guest memory; never trust ptr/len.
    let request = memory
        .data(&caller)
        .get(ptr..ptr.checked_add(len).ok_or_else(|| Error::msg("overflow"))?)
        .ok_or_else(|| Error::msg("host_call request out of bounds"))?
        .to_vec();
    caller.data_mut().host_calls += 1;
    let reply = (caller.data_mut().handler)(&request);
    // The deadline covers continuous guest execution, so time spent in the
    // host (e.g. waiting for the broker) does not count against it. Reset
    // before re-entering the guest (guest_alloc below is guest code).
    if let Some(ticks) = caller.data().epoch_ticks {
        caller.as_context_mut().set_epoch_deadline(ticks);
    }
    let alloc = caller
        .get_export("guest_alloc")
        .and_then(|e| e.into_func())
        .ok_or_else(|| Error::msg("guest has no guest_alloc"))?
        .typed::<u32, u32>(&caller)?;
    let reply_ptr = alloc.call(&mut caller, reply.len() as u32)?;
    memory.write(&mut caller, reply_ptr as usize, &reply)?;
    Ok(((reply_ptr as u64) << 32) | reply.len() as u64)
}

pub struct Plugin {
    pub store: Store<HostState>,
    pub instance: Instance,
}

impl Plugin {
    pub fn call<P, R>(&mut self, name: &str, params: P) -> Result<R>
    where
        P: wasmtime::WasmParams,
        R: wasmtime::WasmResults,
    {
        let f = self
            .instance
            .get_typed_func::<P, R>(&mut self.store, name)?;
        f.call(&mut self.store, params)
    }

    /// Copies `request` into guest memory and calls `export(ptr, len, extra..)`.
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<u32> {
        let ptr: u32 = self.call("guest_alloc", bytes.len() as u32)?;
        let memory = self
            .instance
            .get_memory(&mut self.store, "memory")
            .ok_or_else(|| Error::msg("no memory"))?;
        memory.write(&mut self.store, ptr as usize, bytes)?;
        Ok(ptr)
    }

    fn read_packed(&mut self, packed: u64) -> Result<Vec<u8>> {
        let (ptr, len) = ((packed >> 32) as usize, (packed & 0xFFFF_FFFF) as usize);
        let memory = self
            .instance
            .get_memory(&mut self.store, "memory")
            .ok_or_else(|| Error::msg("no memory"))?;
        memory
            .data(&self.store)
            .get(ptr..ptr + len)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| Error::msg("reply out of bounds"))
    }

    /// Asks the guest to forward `request` through `host_call` once.
    pub fn relay(&mut self, request: &[u8]) -> Result<Vec<u8>> {
        let ptr = self.write_bytes(request)?;
        let packed: u64 = self.call("relay", (ptr, request.len() as u32))?;
        self.read_packed(packed)
    }

    /// Asks the guest to forward `request` `n` times.
    pub fn relay_n(&mut self, request: &[u8], n: u32) -> Result<u32> {
        let ptr = self.write_bytes(request)?;
        self.call("relay_n", (ptr, request.len() as u32, n))
    }

    pub fn memory_bytes(&mut self) -> usize {
        self.instance
            .get_memory(&mut self.store, "memory")
            .map(|m| m.data_size(&self.store))
            .unwrap_or(0)
    }
}

/// Increments the engine epoch every [`TICK`] on a background thread.
pub struct EpochTicker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl EpochTicker {
    pub fn start(engine: &Engine) -> Self {
        let engine = engine.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(TICK);
                engine.increment_epoch();
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Returns the trap code if `err` is a wasm trap.
pub fn trap_of(err: &Error) -> Option<Trap> {
    err.downcast_ref::<Trap>().copied()
}
