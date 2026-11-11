use std::collections::BTreeSet;

use deckpilot_core::notify::RecordingNotifier;
use deckpilot_core::{
    CancellationToken, ExecutionControl, MachineConfig, Os, Profile, Progress, Workflow,
};

use super::*;

fn handlers_with(config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
    NotificationsModule::new().action_handlers(config)
}

fn handler(kind: &str, config: &ActionParameters) -> Box<dyn ActionHandler> {
    handlers_with(config)
        .into_iter()
        .find(|h| h.definition().id.as_str() == kind)
        .unwrap()
}

fn run(
    kind: &str,
    config: &ActionParameters,
    parameters: &ActionParameters,
    context: &ExecutionContext<'_>,
) -> ActionResult {
    let handler = handler(kind, config);
    let normalized = handler.definition().normalize(parameters).unwrap();
    handler.execute(&normalized, context)
}

fn text(value: &str) -> ParameterValue {
    ParameterValue::Text(value.into())
}

fn show(title: &str) -> ActionParameters {
    ActionParameters::new().with("title", text(title))
}

#[test]
fn shows_plain_text_with_the_module_preferences() {
    let profile = Profile::new("Work");
    let machine = MachineConfig::new(Os::current());
    let notifier = RecordingNotifier::new();
    let context = ExecutionContext::new(&profile, None, &machine).with_notifier(&notifier);
    let result = run(
        SHOW,
        &ActionParameters::new().with(DESKTOP, ParameterValue::Boolean(false)),
        &show("Deploy\u{7} done")
            .with("body", text("<b>main</b> is green"))
            .with("severity", ParameterValue::Choice("success".into())),
        &context,
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["shown"], "true");
    let shown = notifier.shown();
    assert_eq!(shown[0].title, "Deploy done");
    // Markup is text; the desktop adapter escapes it where needed.
    assert_eq!(shown[0].body, "<b>main</b> is green");
    assert_eq!(shown[0].severity, Severity::Success);
    assert!(!shown[0].desktop && shown[0].status_bar);
    // The body is not repeated in the node's message (history).
    assert!(!result.message.contains("green"));
}

#[test]
fn failures_do_not_stop_the_workflow_unless_critical() {
    let profile = Profile::new("Work");
    let machine = MachineConfig::new(Os::current());
    let denied = RecordingNotifier::failing();
    let context = ExecutionContext::new(&profile, None, &machine).with_notifier(&denied);
    let quiet = run(SHOW, &ActionParameters::new(), &show("Hello"), &context);
    assert!(quiet.is_success());
    assert_eq!(quiet.data["shown"], "false");
    assert!(quiet.data["reason"].contains("turned off"));
    let critical = run(
        SHOW,
        &ActionParameters::new(),
        &show("Hello").with("critical", ParameterValue::Boolean(true)),
        &context,
    );
    assert_eq!(critical.code.as_deref(), Some(codes::NOT_SHOWN));

    // No notifier at all, and a module without the capability.
    let bare = ExecutionContext::new(&profile, None, &machine);
    assert_eq!(
        run(SHOW, &ActionParameters::new(), &show("x"), &bare).data["shown"],
        "false"
    );
    let notifier = RecordingNotifier::new();
    let granted = BTreeSet::from([Capability::ProfileRead]);
    let restricted = ExecutionContext::new(&profile, None, &machine)
        .with_notifier(&notifier)
        .restricted(&granted);
    let refused = run(SHOW, &ActionParameters::new(), &show("x"), &restricted);
    assert_eq!(refused.data["shown"], "false");
    assert_eq!(notifier.shown(), []);
}

#[test]
fn less_important_notifications_are_filtered() {
    let profile = Profile::new("Work");
    let machine = MachineConfig::new(Os::current());
    let notifier = RecordingNotifier::new();
    let context = ExecutionContext::new(&profile, None, &machine).with_notifier(&notifier);
    let config =
        ActionParameters::new().with(MINIMUM_SEVERITY, ParameterValue::Choice("warning".into()));
    let info = run(SHOW, &config, &show("FYI"), &context);
    assert!(info.is_success());
    assert_eq!(info.data["shown"], "false");
    let error = run(
        SHOW,
        &config,
        &show("Broken").with("severity", ParameterValue::Choice("error".into())),
        &context,
    );
    assert_eq!(error.data["shown"], "true");
    assert_eq!(notifier.shown().len(), 1);
    let off = ActionParameters::new()
        .with(DESKTOP, ParameterValue::Boolean(false))
        .with(STATUS_BAR, ParameterValue::Boolean(false));
    assert_eq!(run(SHOW, &off, &show("x"), &context).data["shown"], "false");
}

#[test]
fn summaries_report_the_run_without_messages() {
    let mut profile = Profile::new("Work");
    let workflow = Workflow::new("Start work");
    let workflow_id = workflow.id;
    profile.workflows.push(workflow);
    let machine = MachineConfig::new(Os::current());
    let notifier = RecordingNotifier::new();
    let token = CancellationToken::new();
    let progress = |_: Progress| {};
    let control = ExecutionControl::new(&token, &progress).with_summary(RunSummary {
        workflow: Some(workflow_id),
        elapsed_ms: 12_300,
        succeeded: 3,
        failed: 1,
        skipped: 2,
        cancelled: 0,
        failed_nodes: vec!["compose".into()],
    });
    let context = ExecutionContext::new(&profile, None, &machine)
        .with_notifier(&notifier)
        .with_control(&control);
    let result = run(
        WORKFLOW_SUMMARY,
        &ActionParameters::new(),
        &ActionParameters::new().with("details", text("Branch: main")),
        &context,
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(
        result.data["summary"],
        "3 succeeded, 1 failed (compose), 2 skipped · 12.3 s"
    );
    let shown = &notifier.shown()[0];
    assert_eq!(shown.title, "Start work");
    assert_eq!(shown.severity, Severity::Error);
    assert_eq!(
        shown.body,
        "3 succeeded, 1 failed (compose), 2 skipped · 12.3 s\nBranch: main"
    );

    // Only on failure: a clean run shows nothing.
    let clean = ExecutionControl::new(&token, &progress).with_summary(RunSummary {
        workflow: Some(workflow_id),
        succeeded: 4,
        ..RunSummary::default()
    });
    let context = ExecutionContext::new(&profile, None, &machine)
        .with_notifier(&notifier)
        .with_control(&clean);
    let skipped = run(
        WORKFLOW_SUMMARY,
        &ActionParameters::new(),
        &ActionParameters::new().with("onlyOnFailure", ParameterValue::Boolean(true)),
        &context,
    );
    assert_eq!(skipped.data["shown"], "false");
    assert_eq!(notifier.shown().len(), 1);
    assert_eq!(
        describe(&RunSummary {
            elapsed_ms: 75_000,
            succeeded: 1,
            ..RunSummary::default()
        }),
        "1 succeeded · 1 min 15 s"
    );
}

#[test]
fn the_node_contract() {
    for handler in handlers_with(&ActionParameters::new()) {
        let node = &handler.definition().node;
        assert_eq!(node.effect, deckpilot_core::graph::ActionEffect::Opens);
        assert!(node.retry_safe && !node.idempotent);
        assert!(node.outputs.iter().any(|o| o.key == "shown"));
        assert!(node.outputs.iter().all(|o| !o.sensitive));
    }
}
