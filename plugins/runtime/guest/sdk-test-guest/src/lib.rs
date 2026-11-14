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
}

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

fn storage(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    ctx.storage_set("count", 3)?;
    let v = ctx.storage_get("count")?;
    Ok(Output::ok().with("found", v.is_some()))
}
