//! wasmtime host for guest ABI 1.
//!
//! The linker defines exactly `deckpilot_v1.host_call`; no WASI is linked,
//! and a module importing anything else (or `host_call` with another type)
//! is rejected before instantiation. One Store and one instance live for the
//! whole life of the process; any fault ends it (the caller exits).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use deckpilot_plugin_abi::guest::{
    EXPORT_ABI_VERSION, EXPORT_ALLOC, EXPORT_INVOKE, EXPORT_MEMORY, IMPORT_HOST_CALL,
    IMPORT_MODULE, InvokeInput, pack, unpack,
};
use deckpilot_plugin_abi::limits::{
    MAX_GUEST_INPUT, MAX_GUEST_OUTPUT, MAX_HOST_CALL_REPLY, MAX_HOST_CALL_REQUEST,
    MAX_HOST_CALLS_PER_INVOCATION, MAX_MODULE_BYTES,
};
use deckpilot_plugin_abi::protocol::{FatalReason, GuestLimits};
use deckpilot_plugin_abi::value::{validate_name, validate_params};
use deckpilot_plugin_abi::{
    ABI_VERSION, ActionOutcome, CallErrorCode, CallResult, CapabilityRequest, Params,
};
use serde_json::Value;
use wasmtime::{
    AsContextMut, Caller, Config, Engine, ExternType, Instance, Linker, Memory, Module, OptLevel,
    Store, StoreLimits, StoreLimitsBuilder, Trap, TypedFunc, ValType,
};

/// Period of the epoch ticker; deadlines are counted in these ticks.
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

/// Hard maxima. Limits from the supervisor are clamped to these.
pub mod hard {
    pub const MAX_DEADLINE_MS: u64 = 10_000;
    pub const MAX_FUEL: u64 = 50_000_000_000;
    pub const MAX_MEMORY_BYTES: u64 = 128 << 20;
    pub const MAX_TABLE_ELEMENTS: u64 = 100_000;
    /// Native stack the wasm code may use.
    pub const MAX_WASM_STACK: usize = 512 << 10;
}

/// The engine name reported in `ready`.
pub const ENGINE_NAME: &str = "wasmtime-50.0.1/cranelift";

/// Effective limits after clamping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveLimits {
    pub deadline_ticks: u64,
    pub fuel: u64,
    pub memory_bytes: usize,
    pub table_elements: usize,
}

impl EffectiveLimits {
    pub fn clamp(l: &GuestLimits) -> Self {
        let ms = l.deadline_ms.clamp(1, hard::MAX_DEADLINE_MS);
        let tick = EPOCH_TICK.as_millis() as u64;
        Self {
            deadline_ticks: ms.div_ceil(tick).max(1),
            fuel: l.fuel_per_invocation.clamp(1, hard::MAX_FUEL),
            memory_bytes: l.memory_bytes.min(hard::MAX_MEMORY_BYTES) as usize,
            table_elements: l.table_elements.min(hard::MAX_TABLE_ELEMENTS) as usize,
        }
    }
}

/// Receives the guest's capability requests (the channel to the supervisor).
pub trait HostCallHandler {
    fn call(
        &mut self,
        invocation_id: u64,
        call_id: u64,
        request: CapabilityRequest,
    ) -> Result<CallResult, ChannelError>;
}

/// The channel to the supervisor failed or the supervisor broke protocol.
#[derive(Debug)]
pub struct ChannelError(pub String);

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "channel: {}", self.0)
    }
}

impl std::error::Error for ChannelError {}

/// The guest broke guest ABI 1.
#[derive(Debug)]
struct GuestAbiError(String);

impl fmt::Display for GuestAbiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "guest ABI violation: {}", self.0)
    }
}

impl std::error::Error for GuestAbiError {}

/// Why the plugin cannot continue. Every fault ends the host process.
#[derive(Debug)]
pub enum Fault {
    Load(String),
    Trap(String),
    Deadline,
    OutOfFuel,
    GuestAbi(String),
    Channel(String),
}

impl Fault {
    pub fn reason(&self) -> FatalReason {
        match self {
            Self::Load(_) => FatalReason::LoadFailed,
            Self::Trap(_) => FatalReason::Trap,
            Self::Deadline => FatalReason::Deadline,
            Self::OutOfFuel => FatalReason::OutOfFuel,
            Self::GuestAbi(_) => FatalReason::GuestAbiViolation,
            Self::Channel(_) => FatalReason::ProtocolViolation,
        }
    }

    fn from_runtime(e: wasmtime::Error) -> Self {
        if let Some(t) = e.downcast_ref::<Trap>() {
            return match t {
                Trap::Interrupt => Self::Deadline,
                Trap::OutOfFuel => Self::OutOfFuel,
                other => Self::Trap(format!("{other:?}")),
            };
        }
        if let Some(g) = e.downcast_ref::<GuestAbiError>() {
            return Self::GuestAbi(g.0.clone());
        }
        if let Some(c) = e.downcast_ref::<ChannelError>() {
            return Self::Channel(c.0.clone());
        }
        Self::Trap(format!("{e:#}"))
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(m) => write!(f, "load failed: {m}"),
            Self::Trap(m) => write!(f, "trap: {m}"),
            Self::Deadline => f.write_str("deadline exceeded"),
            Self::OutOfFuel => f.write_str("out of fuel"),
            Self::GuestAbi(m) => write!(f, "guest ABI violation: {m}"),
            Self::Channel(m) => write!(f, "channel: {m}"),
        }
    }
}

struct HostState {
    limits: StoreLimits,
    handler: Box<dyn HostCallHandler>,
    deadline_ticks: u64,
    invocation_id: u64,
    calls_this_invocation: u32,
    next_call_id: u64,
}

/// The wasmtime engine plus the epoch ticker thread. Create it **before**
/// the OS sandbox is applied, so that no thread is started afterwards.
pub struct PluginEngine {
    engine: Engine,
    _ticker: EpochTicker,
}

impl fmt::Debug for PluginEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginEngine").finish_non_exhaustive()
    }
}

impl PluginEngine {
    pub fn new() -> Result<Self, Fault> {
        let mut config = Config::new();
        config
            .consume_fuel(true)
            .epoch_interruption(true)
            .max_wasm_stack(hard::MAX_WASM_STACK)
            .wasm_multi_memory(false)
            .wasm_memory64(false)
            // No memfd/CoW images: keeps the syscall surface after the
            // sandbox minimal and loading deterministic.
            .memory_init_cow(false)
            .cranelift_opt_level(OptLevel::Speed);
        let engine = Engine::new(&config).map_err(|e| Fault::Load(format!("engine: {e:#}")))?;
        let ticker = EpochTicker::start(&engine);
        Ok(Self {
            engine,
            _ticker: ticker,
        })
    }

    /// Validates, compiles and instantiates a module. The start function and
    /// `dp_abi_version` run under the deadline and fuel limits.
    pub fn load(
        &self,
        bytes: &[u8],
        limits: &GuestLimits,
        handler: Box<dyn HostCallHandler>,
    ) -> Result<LoadedPlugin, Fault> {
        let limits = EffectiveLimits::clamp(limits);
        if bytes.len() > MAX_MODULE_BYTES {
            return Err(Fault::Load("module too large".into()));
        }
        let module = Module::new(&self.engine, bytes).map_err(|e| Fault::Load(format!("{e:#}")))?;
        check_imports(&module)?;
        check_exports(&module)?;

        let mut linker: Linker<HostState> = Linker::new(&self.engine);
        linker
            .func_wrap(IMPORT_MODULE, IMPORT_HOST_CALL, host_call)
            .map_err(|e| Fault::Load(format!("{e:#}")))?;

        let state = HostState {
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                .table_elements(limits.table_elements)
                .instances(1)
                .tables(1)
                .memories(1)
                .trap_on_grow_failure(false)
                .build(),
            handler,
            deadline_ticks: limits.deadline_ticks,
            invocation_id: 0,
            calls_this_invocation: 0,
            next_call_id: 0,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(limits.fuel)
            .map_err(|e| Fault::Load(format!("{e:#}")))?;
        store.set_epoch_deadline(limits.deadline_ticks);
        store.epoch_deadline_trap();

        let instance =
            linker
                .instantiate(&mut store, &module)
                .map_err(|e| match Fault::from_runtime(e) {
                    Fault::Trap(m) => Fault::Load(m),
                    other => other,
                })?;
        let typed = |store: &mut Store<HostState>| -> Result<_, wasmtime::Error> {
            let version = instance.get_typed_func::<(), i32>(&mut *store, EXPORT_ABI_VERSION)?;
            let alloc = instance.get_typed_func::<i32, i32>(&mut *store, EXPORT_ALLOC)?;
            let invoke = instance.get_typed_func::<(i32, i32), i64>(&mut *store, EXPORT_INVOKE)?;
            Ok((version, alloc, invoke))
        };
        let (version, alloc, invoke) =
            typed(&mut store).map_err(|e| Fault::Load(format!("{e:#}")))?;
        let memory = instance
            .get_memory(&mut store, EXPORT_MEMORY)
            .ok_or_else(|| Fault::Load("no memory export".into()))?;

        store.set_epoch_deadline(limits.deadline_ticks);
        let v = version.call(&mut store, ()).map_err(Fault::from_runtime)?;
        if v != ABI_VERSION as i32 {
            return Err(Fault::Load(format!(
                "guest ABI version {v} is not supported"
            )));
        }
        Ok(LoadedPlugin {
            store,
            _instance: instance,
            memory,
            alloc,
            invoke,
            limits,
        })
    }
}

fn check_imports(module: &Module) -> Result<(), Fault> {
    for import in module.imports() {
        let ok = import.module() == IMPORT_MODULE
            && import.name() == IMPORT_HOST_CALL
            && match import.ty() {
                ExternType::Func(f) => {
                    let params: Vec<ValType> = f.params().collect();
                    let results: Vec<ValType> = f.results().collect();
                    params.len() == 2
                        && params.iter().all(|p| matches!(p, ValType::I32))
                        && results.len() == 1
                        && matches!(results[0], ValType::I64)
                }
                _ => false,
            };
        if !ok {
            return Err(Fault::Load(format!(
                "forbidden import {}.{}",
                truncate(import.module(), 64),
                truncate(import.name(), 64)
            )));
        }
    }
    Ok(())
}

fn check_exports(module: &Module) -> Result<(), Fault> {
    let mut has_memory = false;
    for export in module.exports() {
        if export.name() == EXPORT_MEMORY {
            has_memory =
                matches!(export.ty(), ExternType::Memory(m) if !m.is_shared() && !m.is_64());
        }
    }
    if !has_memory {
        return Err(Fault::Load("missing or invalid memory export".into()));
    }
    for name in [EXPORT_ABI_VERSION, EXPORT_ALLOC, EXPORT_INVOKE] {
        if !matches!(module.get_export(name), Some(ExternType::Func(_))) {
            return Err(Fault::Load(format!("missing export {name}")));
        }
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// A loaded plugin: one Store, one instance, for the life of the process.
pub struct LoadedPlugin {
    store: Store<HostState>,
    _instance: Instance,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    invoke: TypedFunc<(i32, i32), i64>,
    limits: EffectiveLimits,
}

impl fmt::Debug for LoadedPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadedPlugin")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl LoadedPlugin {
    pub fn limits(&self) -> EffectiveLimits {
        self.limits
    }

    /// Current linear memory size in bytes.
    pub fn memory_size(&self) -> usize {
        self.memory.data_size(&self.store)
    }

    /// Runs one invocation. `Ok` covers every result the guest produced,
    /// including `invalid-result` for output that breaks ABI 1; `Err` is a
    /// fault after which the caller must not use this plugin again (the
    /// host exits).
    pub fn invoke(
        &mut self,
        invocation_id: u64,
        action: &str,
        params: &Params,
    ) -> Result<ActionOutcome, Fault> {
        if validate_name(action).is_err() || validate_params(params).is_err() {
            return Ok(ActionOutcome::failed(
                "invalid-params",
                "action or parameters out of bounds",
            ));
        }
        let input = serde_json::to_vec(&InvokeInput {
            action: action.to_owned(),
            params: params.clone(),
        })
        .map_err(|e| Fault::Channel(e.to_string()))?;
        if input.len() > MAX_GUEST_INPUT {
            return Ok(ActionOutcome::failed(
                "invalid-params",
                "parameters too large",
            ));
        }

        {
            let s = self.store.data_mut();
            s.invocation_id = invocation_id;
            s.calls_this_invocation = 0;
            // Call ids count from 1 within each invocation.
            s.next_call_id = 0;
        }
        self.store
            .set_fuel(self.limits.fuel)
            .map_err(|e| Fault::Trap(format!("{e:#}")))?;

        self.store.set_epoch_deadline(self.limits.deadline_ticks);
        let ptr = self
            .alloc
            .call(&mut self.store, input.len() as i32)
            .map_err(Fault::from_runtime)?;
        if ptr == 0 {
            return Err(Fault::GuestAbi("dp_alloc failed".into()));
        }
        self.memory
            .write(&mut self.store, ptr as u32 as usize, &input)
            .map_err(|_| Fault::GuestAbi("dp_alloc returned an out-of-bounds buffer".into()))?;

        self.store.set_epoch_deadline(self.limits.deadline_ticks);
        let packed = self
            .invoke
            .call(&mut self.store, (ptr, input.len() as i32))
            .map_err(Fault::from_runtime)?;
        Ok(self.read_outcome(packed as u64))
    }

    fn read_outcome(&self, packed: u64) -> ActionOutcome {
        let invalid = |why: &str| ActionOutcome::failed("invalid-result", why.to_owned());
        let (ptr, len) = unpack(packed);
        let (ptr, len) = (ptr as usize, len as usize);
        if len > MAX_GUEST_OUTPUT {
            return invalid("result too large");
        }
        let Some(bytes) = ptr
            .checked_add(len)
            .and_then(|end| self.memory.data(&self.store).get(ptr..end))
        else {
            return invalid("result out of bounds");
        };
        match serde_json::from_slice::<ActionOutcome>(bytes) {
            Ok(o) if o.validate().is_ok() => o,
            Ok(_) => invalid("result breaks a bound"),
            Err(_) => invalid("result does not parse"),
        }
    }
}

const KNOWN_OPS: &[&str] = &[
    "http.fetch",
    "notify",
    "log",
    "storage.get",
    "storage.set",
    "storage.delete",
];

fn abi_error(msg: &str) -> wasmtime::Error {
    wasmtime::Error::new(GuestAbiError(msg.to_owned()))
}

/// `deckpilot_v1.host_call(ptr, len) -> i64`.
fn host_call(mut caller: Caller<'_, HostState>, ptr: i32, len: i32) -> wasmtime::Result<i64> {
    {
        let s = caller.data_mut();
        s.calls_this_invocation += 1;
        if s.calls_this_invocation > MAX_HOST_CALLS_PER_INVOCATION {
            return Err(abi_error("too many host calls in one invocation"));
        }
    }
    let (ptr, len) = (ptr as u32 as usize, len as u32 as usize);
    if len > MAX_HOST_CALL_REQUEST {
        return Err(abi_error("host_call request too large"));
    }
    let memory = caller
        .get_export(EXPORT_MEMORY)
        .and_then(|e| e.into_memory())
        .ok_or_else(|| abi_error("no memory export"))?;
    // Bounds-checked copy; never trust ptr/len.
    let request = ptr
        .checked_add(len)
        .and_then(|end| memory.data(&caller).get(ptr..end))
        .ok_or_else(|| abi_error("host_call request out of bounds"))?
        .to_vec();

    let reply = match parse_request(&request) {
        Ok(req) => {
            let s = caller.data_mut();
            s.next_call_id += 1;
            let (inv, call) = (s.invocation_id, s.next_call_id);
            s.handler
                .call(inv, call, req)
                .map_err(wasmtime::Error::new)?
        }
        Err(reply) => reply,
    };
    let mut bytes = serde_json::to_vec(&reply).unwrap_or_default();
    if bytes.len() > MAX_HOST_CALL_REPLY {
        bytes = serde_json::to_vec(&CallResult::error(CallErrorCode::Failed, "reply too large"))
            .unwrap_or_default();
    }

    // The deadline bounds continuous guest execution; time spent waiting
    // for the supervisor does not count. Reset it before re-entering the
    // guest (dp_alloc is guest code), or the guest traps on re-entry.
    let ticks = caller.data().deadline_ticks;
    caller.as_context_mut().set_epoch_deadline(ticks);
    let alloc = caller
        .get_export(EXPORT_ALLOC)
        .and_then(|e| e.into_func())
        .ok_or_else(|| abi_error("no dp_alloc export"))?
        .typed::<i32, i32>(&caller)?;
    let reply_ptr = alloc.call(&mut caller, bytes.len() as i32)?;
    if reply_ptr == 0 && !bytes.is_empty() {
        return Err(abi_error("dp_alloc failed"));
    }
    memory
        .write(&mut caller, reply_ptr as u32 as usize, &bytes)
        .map_err(|_| abi_error("dp_alloc returned an out-of-bounds buffer"))?;
    Ok(pack(reply_ptr as u32, bytes.len() as u32) as i64)
}

/// A malformed or unknown request gets an error reply; it never reaches the
/// supervisor.
fn parse_request(bytes: &[u8]) -> Result<CapabilityRequest, CallResult> {
    match serde_json::from_slice::<CapabilityRequest>(bytes) {
        Ok(req) => match req.validate() {
            Ok(()) => Ok(req),
            Err(e) => Err(CallResult::error(CallErrorCode::BadRequest, e.to_string())),
        },
        Err(_) => {
            let unknown_op = serde_json::from_slice::<Value>(bytes)
                .ok()
                .and_then(|v| v.get("op").and_then(Value::as_str).map(str::to_owned))
                .is_some_and(|op| !KNOWN_OPS.contains(&op.as_str()));
            Err(if unknown_op {
                CallResult::error(CallErrorCode::UnsupportedOp, "unsupported operation")
            } else {
                CallResult::error(CallErrorCode::BadRequest, "malformed request")
            })
        }
    }
}

/// Increments the engine epoch every [`EPOCH_TICK`] on a background thread.
struct EpochTicker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn start(engine: &Engine) -> Self {
        let engine = engine.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("epoch-ticker".into())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    std::thread::sleep(EPOCH_TICK);
                    engine.increment_epoch();
                }
            })
            .expect("spawn epoch ticker");
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
