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
    Output::ok()
        .with("error", format!("{:?}", e.kind()))
        .with("detail", e.detail())
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
