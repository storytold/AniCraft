//! The wasmi host: loading, instantiating and running a plug-in under resource limits.
//!
//! Host calls use wasmi's *resumable* calls: `anicraft.call` copies the request out of the
//! module's memory and traps with a [`HostCall`] marker; the run loop answers it through the
//! [`Host`] (which borrows the session mutably, so it can't live in the `'static` store) and
//! resumes the module with the response length. Running out of fuel resumes the same way,
//! slice by slice, so the deadline is checked while a module computes.

use serde_json::{Map, Value};
use wasmi::{Caller, Config, EnforcedLimits, Engine, Extern, ExternType, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc, TypedResumableCall, Val, WasmParams, WasmResults};

use crate::manifest::{Kind, MAX_MANIFEST_BYTES, Manifest};
use crate::{Error, Host, Result};

/// The plug-in ABI version this host implements (`ac_abi_version` must return it).
pub const ABI_VERSION: i32 = 1;
/// The only import module a plug-in may use.
pub const IMPORT_MODULE: &str = "anicraft";
/// The imports ABI v1 offers: `(name, params, results)` (all `i32`).
const IMPORTS: [(&str, usize, usize); 2] = [("call", 2, 1), ("response", 2, 1)];

/// Fuel handed out per slice; between slices the host checks the deadline.
const FUEL_SLICE: u64 = 10_000_000;
/// Longest log line kept, in characters.
const MAX_LOG_CHARS: usize = 500;

/// Samples AniCraft takes of each plug-in ease, at `t = i / (EASE_SAMPLES − 1)` (ADR 0088).
pub const EASE_SAMPLES: usize = 257;
/// Values a plug-in ease may reach (as custom eases: overshoot, never unbounded).
pub const MAX_EASE_VALUE: f64 = 10.0;
/// How far a plug-in ease may miss 0 at t = 0 and 1 at t = 1 (the ends are then pinned).
const EASE_END_TOLERANCE: f64 = 1e-3;

/// One plug-in ease, sampled when the plug-in loaded: `values[i]` is the eased progress at
/// `t = i / (values.len() − 1)`, finite, within ±[`MAX_EASE_VALUE`], from exactly 0 to exactly 1.
#[derive(Clone, Debug, PartialEq)]
pub struct SampledCurve {
    /// The ease's id within the plug-in.
    pub id: String,
    /// The name the ease menu shows.
    pub name: String,
    pub values: Vec<f32>,
}

/// Resource limits for loading and running plug-ins.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Largest module accepted.
    pub max_module_bytes: usize,
    /// Cap on a plug-in's linear memory.
    pub max_memory_bytes: usize,
    /// Instructions (fuel) one run may use; loading (start function, manifest) gets a tenth.
    pub fuel: u64,
    /// Deepest call nesting.
    pub max_recursion_depth: usize,
    /// Wall-clock budget for one run, host calls included, in milliseconds. Native builds
    /// only: the web build has no clock in `std`, so the fuel budget alone bounds it there.
    pub wall_time_ms: u64,
    /// Most `anicraft.call` requests in one run.
    pub max_calls: u32,
    /// Largest request a plug-in may send, and largest response it is given.
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    /// All requests of one run together: bounds what a run can add to the document.
    pub max_total_request_bytes: usize,
    /// Largest params object handed to `ac_run`.
    pub max_params_bytes: usize,
    /// Most log lines kept from one run (later ones are counted, not kept).
    pub max_log_lines: usize,
    /// Largest file an exporter may return from `ac_export`.
    pub max_output_bytes: usize,
}

impl Limits {
    /// The limits of one frame-script run (ADR 0024): it runs between two frames of a playing
    /// movie, so it gets a sliver of a command run's budget. The player shares one budget of
    /// this many instructions among every frame script of a frame.
    pub fn frame_script() -> Limits {
        Limits {
            fuel: 20_000_000,
            wall_time_ms: 100,
            max_calls: 1_000,
            max_request_bytes: 64 << 10,
            max_response_bytes: 1 << 20,
            max_total_request_bytes: 1 << 20,
            max_params_bytes: 16 << 10,
            max_log_lines: 50,
            ..Limits::default()
        }
    }

    /// The limits of one export (ADR 0088). It runs on a worker thread with a progress bar and
    /// Cancel, so it may take much longer than a command (about 100 s of computing, 10 minutes
    /// in all); one request per frame, and bigger answers (a busy frame's paths).
    pub fn exporter() -> Limits {
        Limits { fuel: 50_000_000_000, wall_time_ms: 600_000, max_calls: 1_000_000, max_response_bytes: 32 << 20, ..Limits::default() }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_module_bytes: 16 << 20,
            max_memory_bytes: 256 << 20,
            fuel: 1_000_000_000,
            max_recursion_depth: 1024,
            wall_time_ms: 5_000,
            max_calls: 100_000,
            max_request_bytes: 1 << 20,
            max_response_bytes: 8 << 20,
            max_total_request_bytes: 64 << 20,
            max_params_bytes: 64 << 10,
            max_log_lines: 200,
            max_output_bytes: 64 << 20,
        }
    }
}

/// What a finished run did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunReport {
    /// `log` messages, in order (at most [`Limits::max_log_lines`]).
    pub log: Vec<String>,
    /// Log messages dropped beyond the limit.
    pub log_dropped: u32,
    /// Host requests made, and their total size.
    pub calls: u32,
    pub request_bytes: u64,
    /// Instructions used.
    pub fuel: u64,
}

/// A loaded, validated plug-in. Cheap to share (`Arc`): every run gets its own store and
/// instance.
pub struct Plugin {
    manifest: Manifest,
    engine: Engine,
    module: Module,
    limits: Limits,
    size: usize,
    source: Option<std::path::PathBuf>,
    /// An ease plug-in's eases, sampled at load (empty for other kinds).
    eases: Vec<SampledCurve>,
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin").field("manifest", &self.manifest).field("size", &self.size).field("source", &self.source).finish()
    }
}

/// The marker a host import traps with to hand a request to the run loop.
#[derive(Debug)]
struct HostCall;

impl std::fmt::Display for HostCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("host call")
    }
}

impl wasmi::errors::HostError for HostCall {}

/// Store data: the memory limiter and the request/response being exchanged.
struct Ctx {
    limits: StoreLimits,
    max_request: usize,
    /// The request `anicraft.call` copied out, for the run loop to answer.
    pending: Option<Vec<u8>>,
    /// The last response, for `anicraft.response`.
    response: Vec<u8>,
}

/// One live instance: a store with its own memory limit, and the module's exports.
struct Live {
    store: Store<Ctx>,
    instance: Instance,
    memory: Memory,
    /// Instructions used by finished calls.
    used: u64,
}

impl Live {
    fn func<P: WasmParams, R: WasmResults>(&self, name: &str) -> Result<TypedFunc<P, R>> {
        self.instance.get_typed_func::<P, R>(&self.store, name).map_err(|e| Error::Abi(format!("export `{name}`: {e}")))
    }

    /// `len` bytes at `ptr` in the module's memory.
    fn bytes(&self, ptr: u32, len: u32, what: &str) -> Result<&[u8]> {
        let (p, l) = (ptr as usize, len as usize);
        self.memory.data(&self.store).get(p..p.saturating_add(l)).ok_or_else(|| Error::Abi(format!("{what} points outside the module's memory")))
    }
}

/// When a run must stop (native builds: a wall-clock deadline).
struct Deadline {
    #[cfg(not(target_arch = "wasm32"))]
    at: std::time::Instant,
}

impl Deadline {
    fn new(ms: u64) -> Self {
        #[cfg(target_arch = "wasm32")]
        let _ = ms;
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            at: std::time::Instant::now() + std::time::Duration::from_millis(ms),
        }
    }
    fn check(&self) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        if std::time::Instant::now() >= self.at {
            return Err(Error::Limit("time budget".into()));
        }
        Ok(())
    }
}

/// Stops a run its host has cancelled (checked at every request and every fuel slice).
fn cancelled(host: &Option<(&mut dyn Host, &mut RunReport)>) -> Result<()> {
    match host {
        Some((h, _)) if h.cancelled() => Err(Error::Cancelled),
        _ => Ok(()),
    }
}

fn wasm_err(e: wasmi::Error) -> Error {
    if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) { Error::Limit("instruction budget".into()) } else { Error::Trap(e.to_string()) }
}

/// The module's exported memory, from inside a host function.
fn caller_memory(caller: &Caller<'_, Ctx>) -> std::result::Result<Memory, wasmi::Error> {
    caller.get_export("memory").and_then(Extern::into_memory).ok_or_else(|| wasmi::Error::new("the module must export its memory as `memory`"))
}

/// `anicraft.call(ptr, len) -> i32`: copies the request out and hands it to the run loop.
fn host_call(caller: Caller<'_, Ctx>, ptr: i32, len: i32) -> std::result::Result<i32, wasmi::Error> {
    let mut caller = caller;
    let (p, l) = (ptr as u32 as usize, len as u32 as usize);
    if l > caller.data().max_request {
        return Err(wasmi::Error::new(format!("request of {l} bytes is over the {} byte limit", caller.data().max_request)));
    }
    let memory = caller_memory(&caller)?;
    let bytes = memory.data(&caller).get(p..p.saturating_add(l)).ok_or_else(|| wasmi::Error::new("anicraft.call: the request points outside memory"))?.to_vec();
    caller.data_mut().pending = Some(bytes);
    Err(wasmi::Error::host(HostCall))
}

/// `anicraft.response(ptr, cap) -> i32`: copies up to `cap` bytes of the last response.
fn host_response(caller: Caller<'_, Ctx>, ptr: i32, cap: i32) -> std::result::Result<i32, wasmi::Error> {
    let mut caller = caller;
    let memory = caller_memory(&caller)?;
    let (data, ctx) = memory.data_and_store_mut(&mut caller);
    let n = ctx.response.len().min(cap.max(0) as usize);
    let p = ptr as u32 as usize;
    let dst = data.get_mut(p..p.saturating_add(n)).ok_or_else(|| wasmi::Error::new("anicraft.response: the buffer points outside memory"))?;
    dst.copy_from_slice(ctx.response.get(..n).unwrap_or_default());
    Ok(n as i32)
}

impl Plugin {
    /// Validates and loads a module: checks its imports and exports, the ABI version, and
    /// reads the manifest.
    pub fn load(bytes: &[u8], limits: Limits) -> Result<Plugin> {
        if bytes.len() > limits.max_module_bytes {
            return Err(Error::Module(format!("the module is {} bytes (limit {})", bytes.len(), limits.max_module_bytes)));
        }
        if !bytes.starts_with(b"\0asm") {
            return Err(Error::Module("not a WebAssembly binary (it doesn't start with \\0asm)".into()));
        }
        let mut config = Config::default();
        config.consume_fuel(true).enforced_limits(EnforcedLimits::strict()).set_max_recursion_depth(limits.max_recursion_depth.max(16));
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(|e| Error::Module(e.to_string()))?;
        for imp in module.imports() {
            let known = IMPORTS.iter().find(|(n, _, _)| imp.module() == IMPORT_MODULE && imp.name() == *n);
            let Some((_, np, nr)) = known else {
                return Err(Error::Abi(format!(
                    "the module imports `{}.{}`; plug-ins may import only anicraft.call and anicraft.response (no WASI, files, clock or network)",
                    imp.module(),
                    imp.name()
                )));
            };
            let ok = match imp.ty() {
                ExternType::Func(f) => f.params().len() == *np && f.results().len() == *nr && f.params().iter().chain(f.results()).all(|t| *t == wasmi::ValType::I32),
                _ => false,
            };
            if !ok {
                return Err(Error::Abi(format!("`{IMPORT_MODULE}.{}` must be a function ({}) -> i32", imp.name(), vec!["i32"; *np].join(", "))));
            }
        }
        let mut p = Plugin { manifest: placeholder_manifest(), engine, module, limits, size: bytes.len(), source: None, eases: Vec::new() };
        let deadline = Deadline::new(p.limits.wall_time_ms);
        let load_fuel = p.limits.fuel / 10;
        let mut live = p.instantiate(load_fuel)?;
        let version: TypedFunc<(), i32> = live.func("ac_abi_version")?;
        let v = p.call(&mut live, &version, (), load_fuel, &deadline, &mut None, &p.limits)?;
        if v != ABI_VERSION {
            return Err(Error::Abi(format!("ac_abi_version returned {v}; this AniCraft runs ABI version {ABI_VERSION}")));
        }
        let manifest: TypedFunc<(), i64> = live.func("ac_manifest")?;
        let packed = p.call(&mut live, &manifest, (), load_fuel, &deadline, &mut None, &p.limits)? as u64;
        let (ptr, len) = ((packed & 0xffff_ffff) as u32, (packed >> 32) as u32);
        if len as usize > MAX_MANIFEST_BYTES {
            return Err(Error::Manifest(format!("it is {len} bytes (limit {MAX_MANIFEST_BYTES})")));
        }
        p.manifest = Manifest::parse(live.bytes(ptr, len, "ac_manifest")?)?;
        // The other exports must exist with the right types, per kind.
        match p.manifest.kind {
            Kind::Command | Kind::FrameScript => {
                live.func::<i32, i32>("ac_alloc")?;
                live.func::<(i32, i32), i32>("ac_run")?;
            }
            Kind::Exporter => {
                live.func::<i32, i32>("ac_alloc")?;
                live.func::<(i32, i32), i64>("ac_export")?;
            }
            Kind::Ease => p.eases = p.sample_eases(&mut live, load_fuel, &deadline)?,
        }
        Ok(p)
    }

    /// Samples every declared ease through `ac_ease(index, t) -> value` (both `f64`), on what
    /// is left of the loading budget. The ease can't make host calls, so it is a function of
    /// `t` alone; a value that isn't finite, leaves ±[`MAX_EASE_VALUE`] or misses the ends
    /// stops the load, naming the ease, the time and the value.
    fn sample_eases(&self, live: &mut Live, fuel: u64, deadline: &Deadline) -> Result<Vec<SampledCurve>> {
        let f: TypedFunc<(i32, f64), f64> = live.func("ac_ease")?;
        let mut out = Vec::with_capacity(self.manifest.eases.len());
        for (i, decl) in self.manifest.eases.iter().enumerate() {
            let mut values = Vec::with_capacity(EASE_SAMPLES);
            for k in 0..EASE_SAMPLES {
                let t = k as f64 / (EASE_SAMPLES - 1) as f64;
                let left = fuel.saturating_sub(live.used);
                let v = self.call(live, &f, (i as i32, t), left, deadline, &mut None, &self.limits)?;
                let bad = |what: String| Error::Failed(format!("ease “{}” {what} at t = {t:.3}", decl.name));
                if !v.is_finite() {
                    return Err(bad(format!("gave {v}, not a number")));
                }
                if v.abs() > MAX_EASE_VALUE {
                    return Err(bad(format!("gave {v:.3}, outside −{MAX_EASE_VALUE}..{MAX_EASE_VALUE}")));
                }
                let end = if k == 0 { Some(0.0) } else if k == EASE_SAMPLES - 1 { Some(1.0) } else { None };
                if let Some(e) = end
                    && (v - e).abs() > EASE_END_TOLERANCE
                {
                    return Err(bad(format!("gave {v:.3}; an ease starts at 0 and ends at 1, so a tween begins and ends on its keyframes")));
                }
                values.push(end.unwrap_or(v) as f32);
            }
            out.push(SampledCurve { id: decl.id.clone(), name: decl.name.clone(), values });
        }
        Ok(out)
    }

    /// An ease plug-in's eases, as sampled when it loaded (empty for other kinds).
    pub fn eases(&self) -> &[SampledCurve] {
        &self.eases
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn id(&self) -> &str {
        &self.manifest.id
    }
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
    /// Module size in bytes.
    pub fn size(&self) -> usize {
        self.size
    }
    /// The file the module was loaded from, if any.
    pub fn source(&self) -> Option<&std::path::Path> {
        self.source.as_deref()
    }
    pub fn with_source(mut self, source: impl Into<std::path::PathBuf>) -> Self {
        self.source = Some(source.into());
        self
    }

    fn instantiate(&self, fuel: u64) -> Result<Live> {
        let limits = StoreLimitsBuilder::new().memory_size(self.limits.max_memory_bytes).memories(1).tables(4).table_elements(100_000).instances(1).build();
        let ctx = Ctx { limits, max_request: self.limits.max_request_bytes, pending: None, response: Vec::new() };
        let mut store = Store::new(&self.engine, ctx);
        store.limiter(|c| &mut c.limits);
        // The start function (if any) runs on this fuel.
        store.set_fuel(fuel).map_err(wasm_err)?;
        let mut linker = Linker::<Ctx>::new(&self.engine);
        linker.func_wrap(IMPORT_MODULE, "call", host_call).map_err(|e| Error::Abi(e.to_string()))?;
        linker.func_wrap(IMPORT_MODULE, "response", host_response).map_err(|e| Error::Abi(e.to_string()))?;
        let instance = linker.instantiate_and_start(&mut store, &self.module).map_err(|e| match wasm_err(e) {
            Error::Trap(m) => Error::Module(format!("it can't start: {m}")),
            other => other,
        })?;
        let memory = instance.get_memory(&store, "memory").ok_or_else(|| Error::Abi("the module must export its memory as `memory`".into()))?;
        Ok(Live { store, instance, memory, used: 0 })
    }

    /// Calls `f` with at most `fuel` instructions, in slices, checking the deadline between
    /// them and answering host calls through `host` (`None`: host calls are an error).
    #[allow(clippy::too_many_arguments)]
    fn call<P: WasmParams, R: WasmResults>(&self, live: &mut Live, f: &TypedFunc<P, R>, params: P, fuel: u64, deadline: &Deadline, host: &mut Option<(&mut dyn Host, &mut RunReport)>, limits: &Limits) -> Result<R> {
        let mut left = fuel;
        let give = left.min(FUEL_SLICE);
        left -= give;
        live.store.set_fuel(give).map_err(wasm_err)?;
        let mut call = f.call_resumable(&mut live.store, params).map_err(wasm_err)?;
        loop {
            match call {
                TypedResumableCall::Finished(r) => {
                    live.used = live.used.saturating_add((fuel - left).saturating_sub(live.store.get_fuel().unwrap_or(0)));
                    return Ok(r);
                }
                TypedResumableCall::HostTrap(inv) => {
                    if inv.host_error().downcast_ref::<HostCall>().is_none() {
                        return Err(Error::Trap(inv.host_error().to_string()));
                    }
                    deadline.check()?;
                    cancelled(host)?;
                    let request =live.store.data_mut().pending.take().unwrap_or_default();
                    let response = match host {
                        Some((h, report)) => self.answer(&mut **h, report, &request, limits)?,
                        None => return Err(Error::Abi("host calls are allowed only while the plug-in runs (not while loading)".into())),
                    };
                    let n = response.len() as i32;
                    live.store.data_mut().response = response;
                    call = inv.resume(&mut live.store, &[Val::I32(n)]).map_err(wasm_err)?;
                }
                TypedResumableCall::OutOfFuel(inv) => {
                    let need = inv.required_fuel();
                    if left == 0 || need > left {
                        return Err(Error::Limit("instruction budget".into()));
                    }
                    deadline.check()?;
                    cancelled(host)?;
                    let give = left.min(FUEL_SLICE.max(need));
                    left -= give;
                    live.store.set_fuel(give).map_err(wasm_err)?;
                    call = inv.resume(&mut live.store).map_err(wasm_err)?;
                }
            }
        }
    }

    /// Answers one request: `log` here, everything else through the host. Always a JSON
    /// envelope; only a broken channel (too many calls) stops the run.
    fn answer(&self, host: &mut dyn Host, report: &mut RunReport, request: &[u8], limits: &Limits) -> Result<Vec<u8>> {
        report.calls = report.calls.saturating_add(1);
        if report.calls > limits.max_calls {
            return Err(Error::Limit(format!("host-call budget ({} requests)", limits.max_calls)));
        }
        report.request_bytes = report.request_bytes.saturating_add(request.len() as u64);
        if report.request_bytes > limits.max_total_request_bytes as u64 {
            return Err(Error::Limit(format!("request budget ({} KiB per run)", limits.max_total_request_bytes >> 10)));
        }
        let result: std::result::Result<Value, String> = match serde_json::from_slice::<Value>(request) {
            Err(e) => Err(format!("the request is not JSON: {e}")),
            Ok(req) => match req.get("op").and_then(Value::as_str) {
                None => Err("the request needs an \"op\" (command, document, elements or log)".into()),
                Some("log") => {
                    let msg = req.get("message").map(|m| m.as_str().map_or_else(|| m.to_string(), str::to_string)).unwrap_or_default();
                    if report.log.len() < limits.max_log_lines {
                        report.log.push(msg.chars().filter(|c| !c.is_control()).take(MAX_LOG_CHARS).collect());
                    } else {
                        report.log_dropped = report.log_dropped.saturating_add(1);
                    }
                    Ok(Value::Null)
                }
                Some(op) => host.request(op, &req),
            },
        };
        // `ok` first, so a plug-in can tell success from the first bytes.
        let envelope = |ok: bool, v: &Value| {
            let body = serde_json::to_string(v).unwrap_or_else(|_| "null".into());
            if ok { format!("{{\"ok\":true,\"result\":{body}}}") } else { format!("{{\"ok\":false,\"error\":{body}}}") }.into_bytes()
        };
        let bytes = match &result {
            Ok(v) => envelope(true, v),
            Err(e) => envelope(false, &Value::String(e.clone())),
        };
        if bytes.len() > limits.max_response_bytes {
            let msg = format!("the response is {} bytes, over the {} byte limit; ask for less (e.g. without paths)", bytes.len(), limits.max_response_bytes);
            return Ok(envelope(false, &Value::String(msg)));
        }
        Ok(bytes)
    }

    /// Runs the plug-in once in a fresh instance with validated `params` (see
    /// [`Manifest::resolve_params`]). The host answers its requests; any failure is an error
    /// and the caller should discard what the run changed.
    pub fn run(&self, host: &mut dyn Host, params: &Map<String, Value>) -> Result<RunReport> {
        self.run_with(host, params, &self.limits)
    }

    /// [`Plugin::run`] under other run limits (fuel, time, traffic, log); the module's memory
    /// cap stays the one it was loaded with. Frame scripts run with [`Limits::frame_script`].
    pub fn run_with(&self, host: &mut dyn Host, params: &Map<String, Value>, limits: &Limits) -> Result<RunReport> {
        let deadline = Deadline::new(limits.wall_time_ms);
        let text = serde_json::to_vec(params).map_err(|e| Error::Params(e.to_string()))?;
        if text.len() > limits.max_params_bytes {
            return Err(Error::Params(format!("{} bytes of parameters (limit {})", text.len(), limits.max_params_bytes)));
        }
        let mut live = self.instantiate(limits.fuel)?;
        let alloc: TypedFunc<i32, i32> = live.func("ac_alloc")?;
        let run: TypedFunc<(i32, i32), i32> = live.func("ac_run")?;
        let mut report = RunReport::default();
        let ptr = self.call(&mut live, &alloc, text.len().max(1) as i32, limits.fuel / 10, &deadline, &mut None, limits)? as u32;
        if ptr == 0 {
            return Err(Error::Failed("ac_alloc returned 0 (out of memory)".into()));
        }
        let (p, l) = (ptr as usize, text.len());
        live.memory.data_mut(&mut live.store).get_mut(p..p.saturating_add(l)).ok_or_else(|| Error::Abi("ac_alloc returned a block outside memory".into()))?.copy_from_slice(&text);
        live.used = 0;
        let rc = {
            let mut h: Option<(&mut dyn Host, &mut RunReport)> = Some((host, &mut report));
            self.call(&mut live, &run, (ptr as i32, l as i32), limits.fuel, &deadline, &mut h, limits)
        };
        report.fuel = live.used;
        let rc = rc?;
        if rc != 0 {
            let why = report.log.last().map_or_else(String::new, |m| format!(": {m}"));
            return Err(Error::Failed(format!("{} returned error code {rc}{why}", self.manifest.name)));
        }
        Ok(report)
    }
}

impl Plugin {
    /// Runs an exporter once in a fresh instance (ADR 0088): `ac_export(params_ptr, params_len)`
    /// gets the validated options and returns `(len << 32) | ptr` of the file's bytes, or a
    /// negative number on failure (the last log line says why). The host answers the frame
    /// requests; `limits` are the run's ([`Limits::exporter`]), and the file may be at most
    /// `limits.max_output_bytes`.
    pub fn export(&self, host: &mut dyn Host, options: &Map<String, Value>, limits: &Limits) -> Result<(Vec<u8>, RunReport)> {
        if self.manifest.kind != Kind::Exporter {
            return Err(Error::Abi(format!("{} is not an exporter", self.manifest.name)));
        }
        let deadline = Deadline::new(limits.wall_time_ms);
        let text = serde_json::to_vec(options).map_err(|e| Error::Params(e.to_string()))?;
        if text.len() > limits.max_params_bytes {
            return Err(Error::Params(format!("{} bytes of options (limit {})", text.len(), limits.max_params_bytes)));
        }
        let mut live = self.instantiate(limits.fuel)?;
        let alloc: TypedFunc<i32, i32> = live.func("ac_alloc")?;
        let export: TypedFunc<(i32, i32), i64> = live.func("ac_export")?;
        let mut report = RunReport::default();
        let ptr = self.call(&mut live, &alloc, text.len().max(1) as i32, limits.fuel / 10, &deadline, &mut None, limits)? as u32;
        if ptr == 0 {
            return Err(Error::Failed("ac_alloc returned 0 (out of memory)".into()));
        }
        let (p, l) = (ptr as usize, text.len());
        live.memory.data_mut(&mut live.store).get_mut(p..p.saturating_add(l)).ok_or_else(|| Error::Abi("ac_alloc returned a block outside memory".into()))?.copy_from_slice(&text);
        live.used = 0;
        let packed = {
            let mut h: Option<(&mut dyn Host, &mut RunReport)> = Some((host, &mut report));
            self.call(&mut live, &export, (ptr as i32, l as i32), limits.fuel, &deadline, &mut h, limits)
        };
        report.fuel = live.used;
        let packed = packed?;
        if packed < 0 {
            let why = report.log.last().map_or_else(String::new, |m| format!(": {m}"));
            return Err(Error::Failed(format!("{} could not export{why}", self.manifest.name)));
        }
        let (ptr, len) = ((packed as u64 & 0xffff_ffff) as u32, ((packed as u64) >> 32) as u32);
        if len as usize > limits.max_output_bytes {
            return Err(Error::Limit(format!("output size: it returned {} MiB and an export may be at most {} MiB", (len as usize) >> 20, limits.max_output_bytes >> 20)));
        }
        let bytes = live.bytes(ptr, len, "the exported file")?.to_vec();
        Ok((bytes, report))
    }
}

fn placeholder_manifest() -> Manifest {
    Manifest {
        id: String::new(),
        name: String::new(),
        version: String::new(),
        kind: crate::Kind::Command,
        description: String::new(),
        author: String::new(),
        selection: crate::SelectionNeed::Optional,
        params: Vec::new(),
        entries: Vec::new(),
        format: None,
        eases: Vec::new(),
    }
}
