//! Native unit tests: the definition agrees with manifest.json, the
//! manifest is valid against the schema, and both actions behave, with the
//! agent replaced by `MockHost`. The comparison with the built-in module
//! through the real agent is `apps/agent/tests/plugins_notifications.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use deckpilot_plugin_sdk::testing::{MockHost, manifest_mismatches};
use deckpilot_plugin_sdk::{ActionOutcome, CallErrorCode, CallResult, CapabilityRequest, Params};
use serde_json::{Value, json};

use super::*;

const MANIFEST: &str = include_str!("../manifest.json");
const SCHEMA: &str = include_str!("../../../schemas/plugin-manifest.schema.json");

fn manifest() -> Value {
    serde_json::from_str(MANIFEST).unwrap()
}

#[test]
fn the_definition_matches_the_manifest() {
    assert_eq!(
        manifest_mismatches(&plugin(), &manifest()),
        Vec::<String>::new()
    );
    plugin().validate().unwrap();
}

#[test]
fn the_manifest_is_valid() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&manifest())
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    // Only the low-tier `notifications` permission, so seeding enables it
    // (§4.2), with the routing the agent reads.
    let permissions = manifest()["permissions"].clone();
    assert_eq!(permissions.as_array().unwrap().len(), 1);
    assert_eq!(permissions[0]["kind"], "notifications");
    assert_eq!(
        permissions[0]["routing"],
        json!({"desktop": "desktop", "statusBar": "statusBar", "minimumSeverity": "minimumSeverity"})
    );
}

/// A stand-in agent: answers `notify` with `delivery` and `run.context`
/// with `run`, and records the notifications.
struct Agent {
    host: MockHost,
    sent: Rc<RefCell<Vec<(String, String, Option<NotifySeverity>)>>>,
}

fn agent(delivery: CallResult, run: Value) -> Agent {
    let sent = Rc::new(RefCell::new(Vec::new()));
    let seen = sent.clone();
    let host = MockHost::new(move |request| match request {
        CapabilityRequest::Notify(n) => {
            seen.borrow_mut()
                .push((n.title.clone(), n.body.clone(), n.severity));
            delivery.clone()
        }
        CapabilityRequest::RunContext(_) => CallResult::Ok(run.clone()),
        other => panic!("unexpected request {other:?}"),
    });
    Agent { host, sent }
}

fn delivered(desktop: bool, status_bar: bool) -> CallResult {
    CallResult::Ok(json!({"shown": true, "desktop": desktop, "statusBar": status_bar}))
}

fn not_shown(reason: &str) -> CallResult {
    CallResult::Ok(json!({"shown": false, "reason": reason}))
}

fn ok(outcome: ActionOutcome) -> (String, Params) {
    match outcome {
        ActionOutcome::Ok { message, data } => (message.unwrap_or_default(), data),
        other => panic!("not ok: {other:?}"),
    }
}

fn data(params: &Params) -> Value {
    serde_json::to_value(params).unwrap()
}

#[test]
fn shows_plain_text_where_the_agent_says() {
    let mut agent = agent(delivered(false, true), json!({"trigger": "app"}));
    let (message, outputs) = ok(agent.host.invoke(
        &plugin(),
        "show",
        json!({"title": "Deploy\u{7} done", "body": "<b>main</b>\tis green", "severity": "success"}),
    ));
    assert_eq!(message, "showed “Deploy done”");
    assert_eq!(
        data(&outputs),
        json!({"shown": true, "desktop": false, "statusBar": true})
    );
    let sent = agent.sent.borrow();
    assert_eq!(
        sent[0],
        (
            "Deploy done".to_owned(),
            "<b>main</b> is green".to_owned(),
            Some(NotifySeverity::Success)
        )
    );
    // The body is not repeated in the node's message (history).
    assert!(!message.contains("green"));
}

#[test]
fn long_text_is_cut_to_what_notify_takes() {
    let mut agent = agent(delivered(true, true), json!({"trigger": "app"}));
    let title = "t".repeat(100);
    let (message, _) = ok(agent.host.invoke(
        &plugin(),
        "show",
        json!({"title": title, "body": "b".repeat(500)}),
    ));
    // The message names the built-in's title (100 characters) ...
    assert_eq!(message, format!("showed “{title}”"));
    // ... the agent gets what `notify` takes.
    let (sent_title, sent_body, severity) = agent.sent.borrow()[0].clone();
    assert_eq!(sent_title.chars().count(), SENT_TITLE);
    assert!(sent_title.ends_with('…'));
    assert_eq!(sent_body.chars().count(), SENT_BODY);
    assert_eq!(severity, Some(NotifySeverity::Info));
}

#[test]
fn not_shown_goes_on_unless_critical() {
    let reason = "the app is not open and desktop notifications are off";
    let mut quiet = agent(not_shown(reason), json!({"trigger": "app"}));
    let (message, outputs) = ok(quiet
        .host
        .invoke(&plugin(), "show", json!({"title": "Hello"})));
    assert_eq!(message, format!("“Hello” was not shown: {reason}"));
    assert_eq!(
        data(&outputs),
        json!({"shown": false, "desktop": false, "statusBar": false, "reason": reason})
    );
    let mut critical = agent(not_shown(reason), json!({"trigger": "app"}));
    match critical.host.invoke(
        &plugin(),
        "show",
        json!({"title": "Hello", "critical": true}),
    ) {
        ActionOutcome::Failed { code, message, .. } => {
            assert_eq!(code, codes::NOT_SHOWN);
            assert_eq!(
                message.as_deref(),
                Some(format!("“Hello” was not shown: {reason}").as_str())
            );
        }
        other => panic!("{other:?}"),
    }

    // The rate limit and a revoked permission show nothing either.
    for (error, words) in [
        (
            CallResult::error(CallErrorCode::Throttled, "5 per minute"),
            "more than 5 notifications a minute",
        ),
        (
            CallResult::error(CallErrorCode::NotPermitted, "not granted"),
            "may not show notifications",
        ),
    ] {
        let mut refused = agent(error, json!({"trigger": "app"}));
        let (_, outputs) = ok(refused
            .host
            .invoke(&plugin(), "show", json!({"title": "x"})));
        assert_eq!(
            outputs["shown"],
            deckpilot_plugin_sdk::ParamValue::Bool(false)
        );
        assert!(
            data(&outputs)["reason"].as_str().unwrap().contains(words),
            "{outputs:?}"
        );
    }
}

fn run(failed: u32, cancelled: u32) -> Value {
    json!({
        "trigger": "workflow", "cause": "user",
        "run": {"workflow": "Start work", "elapsedMs": 12_300, "succeeded": 3,
                "failed": failed, "skipped": 2, "cancelled": cancelled,
                "failedNodes": if failed > 0 { json!(["compose"]) } else { json!([]) }}
    })
}

#[test]
fn summaries_report_the_run_without_messages() {
    let mut agent = agent(delivered(true, true), run(1, 0));
    let (message, outputs) = ok(agent.host.invoke(
        &plugin(),
        "workflow-summary",
        json!({"details": "Branch: main"}),
    ));
    assert_eq!(message, "showed “Start work”");
    let summary = "3 succeeded, 1 failed (compose), 2 skipped · 12.3 s";
    assert_eq!(
        data(&outputs),
        json!({"shown": true, "desktop": true, "statusBar": true, "summary": summary})
    );
    assert_eq!(
        agent.sent.borrow()[0],
        (
            "Start work".to_owned(),
            format!("{summary}\nBranch: main"),
            Some(NotifySeverity::Error)
        )
    );

    // Severity follows the run; a title replaces the workflow's name.
    let mut cancelled = agent_with(run(0, 1));
    ok(cancelled
        .host
        .invoke(&plugin(), "workflow-summary", json!({"title": "  Done "})));
    assert_eq!(cancelled.sent.borrow()[0].0, "Done", "the title is trimmed");
    assert_eq!(cancelled.sent.borrow()[0].2, Some(NotifySeverity::Warning));
    let mut clean = agent_with(run(0, 0));
    ok(clean.host.invoke(&plugin(), "workflow-summary", json!({})));
    assert_eq!(clean.sent.borrow()[0].2, Some(NotifySeverity::Success));

    // Outside a workflow: no summary, info, "Workflow".
    let mut direct = agent_with(json!({"trigger": "key"}));
    let (_, outputs) = ok(direct.host.invoke(&plugin(), "workflow-summary", json!({})));
    assert_eq!(data(&outputs)["summary"], "");
    assert_eq!(
        direct.sent.borrow()[0],
        (
            "Workflow".to_owned(),
            String::new(),
            Some(NotifySeverity::Info)
        )
    );

    // Only on failure: a clean run shows nothing.
    let mut quiet = agent_with(run(0, 0));
    let (message, outputs) = ok(quiet.host.invoke(
        &plugin(),
        "workflow-summary",
        json!({"onlyOnFailure": true}),
    ));
    assert_eq!(message, "nothing failed, so no summary was shown");
    assert_eq!(
        data(&outputs),
        json!({"shown": false, "desktop": false, "statusBar": false,
               "reason": "nothing failed", "summary": "3 succeeded, 2 skipped · 12.3 s"})
    );
    assert!(quiet.sent.borrow().is_empty());
}

fn agent_with(run: Value) -> Agent {
    agent(delivered(true, true), run)
}

#[test]
fn text_is_made_plain_like_the_core_does() {
    assert_eq!(plain(" a\u{202e}b\tc\u{7} ", 100, false), "ab c");
    assert_eq!(plain("one\ntwo", 100, false), "onetwo");
    assert_eq!(plain("one\ntwo", 100, true), "one\ntwo");
    assert_eq!(plain("abcdef", 4, false), "abc…");
    assert_eq!(severity("error"), NotifySeverity::Error);
    assert_eq!(severity("critical"), NotifySeverity::Info);
    assert_eq!(
        describe(&RunSummary {
            elapsed_ms: 75_000,
            succeeded: 1,
            ..RunSummary::default()
        }),
        "1 succeeded · 1 min 15 s"
    );
}
