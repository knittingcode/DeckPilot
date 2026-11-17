//! SDK test guest: what a sample plugin should not do, plus the SDK
//! features the sample does not use.
#![deny(unsafe_code)]

use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use deckpilot_plugin_sdk::prelude::*;

static INITS: AtomicU32 = AtomicU32::new(0);
static PANIC_ON_SHUTDOWN: AtomicBool = AtomicBool::new(false);

fn plugin() -> Plugin {
    Plugin::new("acme.sdk-test", "1.0.0")
        .on_init(|| {
            INITS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .on_shutdown(|| {
            if PANIC_ON_SHUTDOWN.load(Ordering::SeqCst) {
                panic!("shutdown hook panics on purpose");
            }
        })
        .action(
            Action::new("scale", scale)
                .param(Param::number("x").required().min(-10.0).max(10.0))
                .param(Param::integer("factor").min(1.0).max(4.0).default(2)),
        )
        .action(Action::new("inits", inits))
        .action(Action::new("arm-shutdown-panic", arm_shutdown_panic))
        .action(
            Action::new("authed", authed)
                .param(Param::secret("token").required())
                .param(Param::choice("scheme", &["bearer", "basic"]).default("bearer")),
        )
        .action(Action::new("many-calls", many_calls))
        .action(Action::new("storage", storage))
        .action(Action::new("bad-code", |_, _| {
            Err(PluginError::new("Not A Code", "invalid on purpose"))
        }))
        .action(Action::new("panic", |_, _| {
            panic!("guest panic on purpose")
        }))
        .action(Action::new("spin", |_, _| {
            loop {
                black_box(());
            }
        }))
        .action(Action::new("slow", |ctx, _| {
            let r = ctx.http_get("https://slow.example.com/")?;
            Ok(Output::ok().with("status", r.status))
        }))
        .action(Action::new("publish", publish))
        .action(Action::new("clear", |ctx, _| {
            ctx.clear_variable("status")?;
            Ok(Output::ok())
        }))
        .action(Action::new("fetch-publish", fetch_publish))
        .action(Action::new("two-fetches", two_fetches))
        // Storage, credentials and settings, for the agent's end-to-end
        // test.
        .action(
            Action::new("store-put", store_put)
                .param(Param::text("key").required())
                .param(Param::text("value").required()),
        )
        .action(Action::new("store-get", store_get).param(Param::text("key").required()))
        .action(Action::new("store-delete", store_delete).param(Param::text("key").required()))
        .action(Action::new("store-fill", store_fill).param(Param::integer("kib").required()))
        .action(Action::new("store-usage", store_usage))
        .action(
            Action::new("cred-fetch", cred_fetch)
                .param(Param::text("url").required())
                .param(Param::text("credential").required())
                .param(Param::choice("scheme", &["bearer", "basic", "header", "query"]).required()),
        )
        .action(Action::new("cred-has", cred_has).param(Param::text("credential").required()))
        .action(Action::new("settings", settings))
        // `url.open` and response headers, for the agent's end-to-end test.
        .action(
            Action::new("open-url", open_url)
                .param(Param::text("url").required())
                .param(Param::integer("times").min(1.0).max(10.0).default(1)),
        )
        .action(Action::new("headers", headers).param(Param::text("url").required()))
        // What started the run (G8), and `url.open` only when the user did.
        .action(Action::new("run-context", run_context))
        .action(Action::new("open-if-user", open_if_user).param(Param::text("url").required()))
        // The clock and the random source (plugin API 1.1).
        .action(Action::new("clock", clock))
        .action(
            Action::new("random", random)
                .param(Param::integer("length").min(0.0).max(1000.0).default(16))
                .param(Param::integer("times").min(1.0).max(200.0).default(1)),
        )
        .action(Action::new("dice", dice).param(Param::integer("sides").min(1.0).default(6)))
        // A plain GET whose failure goes through `?`: the code a plugin
        // reports without handling the error itself.
        .action(Action::new("get", get).param(Param::text("url").required()))
}

/// A server that answers late (canned in the agent's tests).
const SLOW_URL: &str = "https://api.open-meteo.com/v1/test/slow";
/// A server that answers much later than an action should wait.
const VERY_SLOW_URL: &str = "https://api.open-meteo.com/v1/test/very-slow";
/// A server that answers at once.
const FAST_URL: &str = "https://api.open-meteo.com/v1/test/fast";

deckpilot_plugin_sdk::plugin!(plugin);

fn scale(_: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let x: f64 = args.get("x")?;
    let factor: i64 = args.get("factor")?;
    Ok(Output::ok().with("y", x * factor as f64))
}

fn inits(_: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    Ok(Output::ok().with("inits", INITS.load(Ordering::SeqCst)))
}

fn arm_shutdown_panic(_: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    PANIC_ON_SHUTDOWN.store(true, Ordering::SeqCst);
    Ok(Output::ok())
}

fn authed(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let token: SecretHandle = args.get("token")?;
    let scheme = match args.get::<String>("scheme")?.as_str() {
        "basic" => AuthScheme::Basic,
        _ => AuthScheme::Bearer,
    };
    let r = ctx.fetch(Request::get("https://api.example.com/me").credential(&token, scheme))?;
    Ok(Output::ok().with("status", r.status))
}

fn many_calls(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    for i in 0..300 {
        ctx.debug(format!("line {i}"));
    }
    // The SDK stops at the per-invocation limit instead of trapping.
    let refused = match ctx.storage_get("k") {
        Err(e) => e.kind() == CapabilityErrorKind::Throttled,
        Ok(_) => false,
    };
    Ok(Output::ok()
        .with("calls", ctx.calls())
        .with("refusedLocally", refused))
}

/// Publishes its declared variables, and tries what must be refused: an
/// undeclared id and a value of the wrong type.
fn publish(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    ctx.publish_variable("status", "ready")?;
    ctx.publish_variable("count", 42)?;
    ctx.publish_variable("online", true)?;
    let refused = [
        ctx.publish_variable("undeclared", 1).is_err(),
        ctx.publish_variable("count", "forty-two").is_err(),
    ]
    .iter()
    .filter(|r| **r)
    .count();
    Ok(Output::ok().with("refused", refused as i64))
}

/// Waits for a very slow server, then publishes what it answered: when the
/// action is cancelled meanwhile, nothing may be published.
fn fetch_publish(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    let r = ctx.http_get(VERY_SLOW_URL)?;
    ctx.publish_variable("lastStatus", i64::from(r.status))?;
    Ok(Output::ok().with("status", r.status))
}

/// Two privileged calls in one invocation: the second one must see a
/// permission revoked while the first was running.
fn two_fetches(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    let first = ctx.http_get(SLOW_URL)?;
    let second = match ctx.http_get(FAST_URL) {
        Ok(r) => format!("status {}", r.status),
        Err(e) if e.is_denied() => "not-permitted".to_owned(),
        Err(e) => format!("{:?}", e.kind()),
    };
    Ok(Output::ok()
        .with("first", first.status)
        .with("second", second))
}

fn storage(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    ctx.storage_set("count", 3)?;
    let v = ctx.storage_get("count")?;
    Ok(Output::ok().with("found", v.is_some()))
}

/// A capability error as data, so the test sees its kind and detail.
fn refused(e: &CapabilityError) -> Output {
    let out = Output::ok()
        .with("error", format!("{:?}", e.kind()))
        .with("detail", e.detail());
    match e.network_kind() {
        Some(kind) => out.with("network", kind.code()),
        None => out,
    }
}

fn get(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let url: String = args.get("url")?;
    let r = ctx.http_get(url)?;
    Ok(Output::ok().with("status", r.status))
}

fn store_put(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let key: String = args.get("key")?;
    let value: String = args.get("value")?;
    if let Err(e) = ctx.storage_set(key, value) {
        return Ok(refused(&e));
    }
    store_usage(ctx, args)
}

fn store_get(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let key: String = args.get("key")?;
    match ctx.storage_get(key) {
        Ok(Some(v)) => Ok(Output::ok().with("found", true).with(
            "value",
            v.as_str().map_or_else(|| v.to_string(), str::to_owned),
        )),
        Ok(None) => Ok(Output::ok().with("found", false)),
        Err(e) => Ok(refused(&e)),
    }
}

fn store_delete(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let key: String = args.get("key")?;
    if let Err(e) = ctx.storage_delete(key) {
        return Ok(refused(&e));
    }
    store_usage(ctx, args)
}

/// Writes about `kib` KiB in 32 KiB values, stopping at the first refusal.
fn store_fill(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let kib: i64 = args.get("kib")?;
    let chunk = "x".repeat(32 * 1024);
    let mut written: i64 = 0;
    for i in 0..(kib + 31) / 32 {
        if let Err(e) = ctx.storage_set(format!("fill.{i}"), &chunk) {
            return Ok(refused(&e).with("written", written));
        }
        written += 1;
    }
    Ok(Output::ok().with("written", written))
}

fn clock(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    let now = match ctx.now() {
        Ok(now) => now,
        Err(e) => return Ok(refused(&e)),
    };
    Ok(Output::ok()
        .with("unixMs", i64::try_from(now.unix_ms).unwrap_or(i64::MAX))
        .with("utcOffsetMinutes", i64::from(now.utc_offset_minutes))
        .with("minuteOfDay", i64::from(now.local_minutes_of_day()))
        .with("weekday", i64::from(now.local_weekday())))
}

/// Asks for `length` random bytes `times` times; reports them as hex and
/// how many calls were answered before one was refused.
fn random(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let length = usize::try_from(args.get::<i64>("length")?).unwrap_or(0);
    let times: i64 = args.get("times")?;
    let mut hex = String::new();
    let mut answered = 0i64;
    let mut refusal = None;
    for _ in 0..times {
        match ctx.random_bytes(length) {
            Ok(bytes) => {
                answered += 1;
                hex = bytes.iter().map(|b| format!("{b:02x}")).collect();
            }
            Err(e) => {
                refusal = Some(e);
                break;
            }
        }
    }
    let out = match &refusal {
        Some(e) => refused(e),
        None => Output::ok(),
    };
    Ok(out.with("hex", hex).with("answered", answered))
}

fn dice(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let sides = u64::try_from(args.get::<i64>("sides")?).unwrap_or(1);
    match ctx.random_below(sides) {
        Ok(n) => Ok(Output::ok().with("roll", i64::try_from(n + 1).unwrap_or(i64::MAX))),
        Err(e) => Ok(refused(&e)),
    }
}

fn store_usage(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    let usage = match ctx.storage_usage() {
        Ok(usage) => usage,
        Err(e) => return Ok(refused(&e)),
    };
    let keys = ctx.storage_keys(None)?;
    let int = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
    Ok(Output::ok()
        .with("usedBytes", int(usage.used_bytes))
        .with("count", int(usage.keys))
        .with("quotaBytes", int(usage.quota_bytes))
        .with("maxKeys", int(usage.max_keys))
        .with("keys", keys.join(",")))
}

fn cred_fetch(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let url: String = args.get("url")?;
    let slot = SecretHandle::slot(&args.get::<String>("credential")?)?;
    let scheme = match args.get::<String>("scheme")?.as_str() {
        "basic" => AuthScheme::Basic,
        "header" => AuthScheme::Header,
        "query" => AuthScheme::Query,
        _ => AuthScheme::Bearer,
    };
    match ctx.fetch(Request::get(url).credential(&slot, scheme)) {
        // Everything the plugin received, so the test can check that the
        // secret is not in it.
        Ok(r) => Ok(Output::ok()
            .with("status", r.status)
            .with("body", r.text())
            .with("headers", format!("{:?}", r.headers))),
        Err(e) => Ok(refused(&e)),
    }
}

fn cred_has(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let slot = SecretHandle::slot(&args.get::<String>("credential")?)?;
    match ctx.has_credential(&slot) {
        Ok(set) => Ok(Output::ok().with("set", set)),
        Err(e) => Ok(refused(&e)),
    }
}

fn settings(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    match ctx.settings() {
        Ok(s) => Ok(Output::ok().with("settings", s.to_json())),
        Err(e) => Ok(refused(&e)),
    }
}

/// Opens `url` up to `times` times, stopping at the first refusal.
fn open_url(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let url: String = args.get("url")?;
    let times: i64 = args.get("times")?;
    let mut opened: i64 = 0;
    for _ in 0..times {
        if let Err(e) = ctx.open_url(url.as_str()) {
            return Ok(refused(&e).with("opened", opened));
        }
        opened += 1;
    }
    Ok(Output::ok().with("opened", opened))
}

/// The invocation's run context, flattened into outputs.
fn run_context(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    let run = match ctx.run_context() {
        Ok(run) => run,
        Err(e) => return Ok(refused(&e)),
    };
    let trigger = match run.trigger {
        RunTrigger::Key => "key",
        RunTrigger::Gesture => "gesture",
        RunTrigger::App => "app",
        RunTrigger::Workflow => "workflow",
        RunTrigger::Test => "test",
        RunTrigger::Other => "other",
    };
    let cause = match run.cause {
        None => "",
        Some(RunCause::User) => "user",
        Some(RunCause::Schedule) => "schedule",
        Some(RunCause::Event) => "event",
        Some(RunCause::Startup) => "startup",
        Some(RunCause::WorkSession) => "workSession",
        Some(RunCause::Other) => "other",
    };
    let mut out = Output::ok()
        .with("trigger", trigger)
        .with("cause", cause)
        .with("userInitiated", run.is_user_initiated());
    if let Some(summary) = &run.run {
        out = out
            .with("workflow", summary.workflow.clone().unwrap_or_default())
            .with("succeeded", i64::from(summary.succeeded))
            .with("failed", i64::from(summary.failed))
            .with("failedNodes", summary.failed_nodes.join(","));
    }
    Ok(out)
}

/// Opens `url` when the user started the run; otherwise says why not
/// without asking (and tries anyway, to show the agent refuses it).
fn open_if_user(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let url: String = args.get("url")?;
    let user = ctx.run_context().is_ok_and(|r| r.is_user_initiated());
    match ctx.open_url(url.as_str()) {
        Ok(()) => Ok(Output::ok().with("opened", 1).with("userInitiated", user)),
        Err(e) => Ok(refused(&e).with("opened", 0).with("userInitiated", user)),
    }
}

/// What a paginated, rate-limited API tells the plugin in its headers.
fn headers(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let url: String = args.get("url")?;
    let r = match ctx.http_get(url) {
        Ok(r) => r,
        Err(e) => return Ok(refused(&e)),
    };
    let names: Vec<String> = r
        .headers
        .iter()
        .map(|h| h.name.to_ascii_lowercase())
        .collect();
    let mut out = Output::ok()
        .with("status", r.status)
        .with("names", names.join(","))
        .with("headers", format!("{:?}", r.headers));
    for (key, value) in [
        ("next", r.link("next")),
        ("last", r.link("last")),
        ("remaining", r.header("x-ratelimit-remaining")),
        ("reset", r.header("ratelimit-reset")),
        ("total", r.header("x-total")),
        ("etag", r.header("etag")),
        ("cookie", r.header("set-cookie")),
    ] {
        if let Some(value) = value {
            out = out.with(key, value);
        }
    }
    Ok(out)
}
