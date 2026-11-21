//! Plugin API 1.9's operations (`docs/marketplace/module-migration.md` §7):
//! each is denied by default, allowed only with its own grant, refused from
//! runs the user did not start where the review says so, and rate limited.

use std::sync::Arc;

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::capability::{NoArgs, RunCause, RunContext, RunTrigger};
use deckpilot_plugin_abi::privileged::{
    CycleDirection, OpenUrlParam, ProjectCycle, ProjectSelect, ResourceKind, ResourceOpen,
    ResourceResolve, Tool, ToolAccess, ToolRun,
};
use deckpilot_plugin_broker::*;

const HANDLE: &str = "00112233445566778899aabbccddeeff";

fn session(perms: &str) -> (Broker, Session, Arc<MemoryAudit>) {
    let audit = Arc::new(MemoryAudit::new(10_000));
    let broker = Broker::new(
        audit.clone(),
        Arc::new(ManualClock::default()),
        RateLimits::default(),
    );
    let plugin = PluginId::new("deckpilot.git").unwrap();
    broker
        .set_grants(&plugin, &ApprovedPermissions::from_json(perms).unwrap())
        .unwrap();
    let session = broker.session(plugin);
    (broker, session, audit)
}

fn resolve() -> CapabilityRequest {
    CapabilityRequest::ResourceResolve(ResourceResolve {
        param: None,
        role: Some("repository".into()),
        project_id: None,
        kinds: vec![ResourceKind::Directory],
    })
}

fn open() -> CapabilityRequest {
    CapabilityRequest::ResourceOpen(ResourceOpen {
        handle: HANDLE.into(),
        path: None,
        with: None,
        position: None,
        editor: None,
    })
}

fn run(tool: Tool, args: &[&str]) -> CapabilityRequest {
    CapabilityRequest::ToolRun(ToolRun {
        tool,
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        cwd: Some(HANDLE.into()),
        timeout_ms: None,
    })
}

fn open_param() -> CapabilityRequest {
    CapabilityRequest::OpenUrlParam(OpenUrlParam {
        param: "urls".into(),
        index: Some(0),
    })
}

fn select() -> CapabilityRequest {
    CapabilityRequest::ProjectSelect(ProjectSelect { id: "p-1".into() })
}

fn cycle() -> CapabilityRequest {
    CapabilityRequest::ProjectCycle(ProjectCycle {
        direction: CycleDirection::Next,
        wrap: None,
    })
}

/// One request of every plugin API 1.9 operation.
fn every_op() -> Vec<CapabilityRequest> {
    vec![
        resolve(),
        open(),
        run(Tool::Git, &["status", "-z"]),
        run(Tool::Git, &["ls-remote", "--", "origin"]),
        run(Tool::Docker, &["ps"]),
        run(Tool::Docker, &["compose", "up", "--detach"]),
        open_param(),
        CapabilityRequest::ProjectCurrent(NoArgs {}),
        CapabilityRequest::ProjectList(NoArgs {}),
        select(),
        cycle(),
        CapabilityRequest::ProfileRead(NoArgs {}),
    ]
}

fn user() -> RunContext {
    RunContext::direct(RunTrigger::Key)
}

fn denial(s: &Session, request: &CapabilityRequest, context: &RunContext) -> Option<DenyReason> {
    s.authorize_in(request, Some(context)).denial()
}

#[test]
fn every_op_is_denied_by_default() {
    // Grants of every other kind, none of 1.9's.
    let (_, s, audit) = session(
        r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET"]},
            {"kind":"openUrl","origin":"https://github.com"},
            {"kind":"notifications"},{"kind":"dialogs"},{"kind":"keyState"}]"#,
    );
    for request in every_op() {
        assert_eq!(
            denial(&s, &request, &user()),
            Some(DenyReason::NotGranted),
            "{}",
            request.op()
        );
    }
    // Each denial is audited under its operation, never with a value.
    let ops: Vec<&str> = audit.records().iter().map(|r| r.op).collect();
    assert!(ops.contains(&"tool.run") && ops.contains(&"project.select"));
    // `tool.run` names its tool and subcommand, nothing else.
    for record in audit.records() {
        match record.op {
            "tool.run" => assert!(
                record
                    .target
                    .as_deref()
                    .is_some_and(|t| t.starts_with("git ") || t.starts_with("docker ")),
                "{record:?}"
            ),
            _ => assert_eq!(record.target, None, "{record:?}"),
        }
    }

    // A plugin without grants at all.
    let broker = Broker::new(
        Arc::new(MemoryAudit::new(10)),
        Arc::new(ManualClock::default()),
        RateLimits::default(),
    );
    let s = broker.session(PluginId::new("deckpilot.other").unwrap());
    for request in every_op() {
        assert_eq!(
            denial(&s, &request, &user()),
            Some(DenyReason::UnknownPlugin)
        );
    }
}

#[test]
fn each_op_is_allowed_only_with_its_own_grant() {
    let (_, s, _) = session(r#"[{"kind":"resources"}]"#);
    let Decision::Allow(Allowed::ResourceResolve { paths, .. }) =
        s.authorize_in(&resolve(), Some(&user()))
    else {
        panic!("resolve");
    };
    assert!(!paths, "no displayPath without resources:paths");
    assert_eq!(denial(&s, &open(), &user()), Some(DenyReason::NotGranted));

    let (_, s, _) = session(r#"[{"kind":"resources","paths":true,"open":true}]"#);
    assert!(matches!(
        s.authorize_in(&resolve(), Some(&user())),
        Decision::Allow(Allowed::ResourceResolve { paths: true, .. })
    ));
    assert!(matches!(
        s.authorize_in(&open(), Some(&user())),
        Decision::Allow(Allowed::ResourceOpen { .. })
    ));

    // tools:git: read only; the network and docker need their own grants.
    let (_, s, _) = session(r#"[{"kind":"tool","tool":"git"}]"#);
    assert!(matches!(
        s.authorize_in(&run(Tool::Git, &["status"]), Some(&user())),
        Decision::Allow(Allowed::ToolRun {
            access: ToolAccess::Read,
            ..
        })
    ));
    for request in [
        run(Tool::Git, &["ls-remote", "--", "origin"]),
        run(Tool::Docker, &["ps"]),
    ] {
        assert_eq!(denial(&s, &request, &user()), Some(DenyReason::NotGranted));
    }
    let (_, s, _) = session(r#"[{"kind":"tool","tool":"git","network":true}]"#);
    assert!(matches!(
        s.authorize_in(
            &run(Tool::Git, &["ls-remote", "--", "origin"]),
            Some(&user())
        ),
        Decision::Allow(Allowed::ToolRun {
            access: ToolAccess::Network,
            ..
        })
    ));

    // tools:docker without modify: reads only.
    let (_, s, _) = session(r#"[{"kind":"tool","tool":"docker"}]"#);
    assert!(
        s.authorize_in(&run(Tool::Docker, &["ps"]), Some(&user()))
            .is_allowed()
    );
    for args in [
        &["compose", "up"][..],
        &["compose", "down"],
        &["stop", "--", "web"],
        &["restart", "web"],
    ] {
        assert_eq!(
            denial(&s, &run(Tool::Docker, args), &user()),
            Some(DenyReason::NotGranted),
            "{args:?}"
        );
    }
    let (_, s, _) = session(r#"[{"kind":"tool","tool":"docker","modify":true}]"#);
    assert!(matches!(
        s.authorize_in(
            &run(Tool::Docker, &["compose", "up", "--detach"]),
            Some(&user())
        ),
        Decision::Allow(Allowed::ToolRun {
            access: ToolAccess::Modify,
            ..
        })
    ));

    let (_, s, _) = session(r#"[{"kind":"openUrlFromParams"}]"#);
    assert!(s.authorize_in(&open_param(), Some(&user())).is_allowed());

    let (_, s, _) = session(r#"[{"kind":"projectContext"}]"#);
    assert!(matches!(
        s.authorize_in(&CapabilityRequest::ProjectCurrent(NoArgs {}), Some(&user())),
        Decision::Allow(Allowed::ProjectCurrent)
    ));
    assert!(matches!(
        s.authorize_in(&CapabilityRequest::ProjectList(NoArgs {}), Some(&user())),
        Decision::Allow(Allowed::ProjectList)
    ));
    for request in [select(), cycle()] {
        assert_eq!(denial(&s, &request, &user()), Some(DenyReason::NotGranted));
    }
    let (_, s, _) = session(r#"[{"kind":"projectContext","select":true}]"#);
    assert!(matches!(
        s.authorize_in(&select(), Some(&user())),
        Decision::Allow(Allowed::ProjectSelect { id }) if id == "p-1"
    ));

    let (_, s, _) = session(r#"[{"kind":"profile","read":["project"]}]"#);
    let Decision::Allow(Allowed::ProfileRead { scopes }) =
        s.authorize_in(&CapabilityRequest::ProfileRead(NoArgs {}), Some(&user()))
    else {
        panic!("profile.read");
    };
    assert_eq!(
        scopes.into_iter().collect::<Vec<_>>(),
        [ProfileScope::Project]
    );
}

#[test]
fn opening_switching_and_modifying_need_a_run_the_user_started() {
    let (_, s, _) = session(
        r#"[{"kind":"resources","open":true},{"kind":"openUrlFromParams"},
            {"kind":"projectContext","select":true},
            {"kind":"tool","tool":"docker","modify":true},{"kind":"tool","tool":"git"}]"#,
    );
    let not_user = [
        RunContext::workflow(RunCause::Schedule, None),
        RunContext::workflow(RunCause::Event, None),
        RunContext::workflow(RunCause::Startup, None),
        RunContext::unknown(),
    ];
    for request in [
        open(),
        open_param(),
        select(),
        cycle(),
        run(Tool::Docker, &["compose", "up"]),
    ] {
        for context in &not_user {
            assert_eq!(
                denial(&s, &request, context),
                Some(DenyReason::PrivilegedNotUserInitiated),
                "{} {context:?}",
                request.op()
            );
        }
        // No context at all is not the user's run either.
        assert_eq!(
            s.authorize(&request).denial(),
            Some(DenyReason::PrivilegedNotUserInitiated)
        );
    }
    // Selecting the project from a work session is refused too; bringing
    // compose up from one the user started is the reviewed exception.
    let session_run = RunContext::workflow(RunCause::WorkSession, None);
    assert_eq!(
        denial(&s, &select(), &session_run),
        Some(DenyReason::PrivilegedNotUserInitiated)
    );
    assert!(
        s.authorize_in(&run(Tool::Docker, &["compose", "up"]), Some(&session_run))
            .is_allowed()
    );
    // A workflow the user started counts as the user's.
    assert!(
        s.authorize_in(&select(), Some(&RunContext::workflow(RunCause::User, None)))
            .is_allowed()
    );
    // Reads are not restricted to the user's runs.
    assert!(
        s.authorize_in(&run(Tool::Git, &["status"]), Some(&not_user[0]))
            .is_allowed()
    );
}

#[test]
fn a_run_must_start_with_a_subcommand() {
    let (_, s, _) = session(r#"[{"kind":"tool","tool":"git","network":true}]"#);
    for args in [
        &["-c", "core.sshCommand=evil", "status"][..],
        &["--exec-path=/tmp"],
        &["-C", "/", "status"],
    ] {
        assert_eq!(
            denial(&s, &run(Tool::Git, args), &user()),
            Some(DenyReason::NoSubcommand),
            "{args:?}"
        );
    }
}

#[test]
fn rates_and_revocation_apply_on_the_next_call() {
    let (broker, s, _) = session(
        r#"[{"kind":"resources","open":true},{"kind":"projectContext","select":true},
            {"kind":"tool","tool":"docker","modify":true}]"#,
    );
    // resource.open: a burst of 5 (one open-project), then 1 per second.
    for _ in 0..5 {
        assert!(s.authorize_in(&open(), Some(&user())).is_allowed());
    }
    assert_eq!(denial(&s, &open(), &user()), Some(DenyReason::RateLimited));
    // One project switch per 500 ms (X6: 2 per second without a burst).
    assert!(s.authorize_in(&select(), Some(&user())).is_allowed());
    assert_eq!(denial(&s, &cycle(), &user()), Some(DenyReason::RateLimited));
    for _ in 0..6 {
        assert!(
            s.authorize_in(&run(Tool::Docker, &["restart", "web"]), Some(&user()))
                .is_allowed()
        );
    }
    assert_eq!(
        denial(&s, &run(Tool::Docker, &["restart", "web"]), &user()),
        Some(DenyReason::RateLimited)
    );
    // Reads keep their own budget.
    assert!(
        s.authorize_in(&run(Tool::Docker, &["ps"]), Some(&user()))
            .is_allowed()
    );

    // New grants without them: denied on the next call.
    broker
        .set_grants(
            s.plugin(),
            &ApprovedPermissions::from_json(r#"[{"kind":"notifications"}]"#).unwrap(),
        )
        .unwrap();
    assert_eq!(
        denial(&s, &run(Tool::Docker, &["ps"]), &user()),
        Some(DenyReason::NotGranted)
    );
    broker.remove_plugin(s.plugin());
    assert_eq!(
        denial(&s, &resolve(), &user()),
        Some(DenyReason::UnknownPlugin)
    );
}

#[test]
fn grant_records_are_strict() {
    for bad in [
        r#"[{"kind":"tool","tool":"bash"}]"#,
        r#"[{"kind":"tool","tool":"git","argv":["x"]}]"#,
        r#"[{"kind":"resources","kinds":["directory"]}]"#,
        r#"[{"kind":"projectContext","write":true}]"#,
        r#"[{"kind":"profile","read":["layouts"]}]"#,
        r#"[{"kind":"openUrlFromParams","hosts":["x"]}]"#,
    ] {
        assert_eq!(
            ApprovedPermissions::from_json(bad),
            Err(GrantError::Malformed),
            "{bad}"
        );
    }
    for (bad, error) in [
        (
            r#"[{"kind":"tool","tool":"docker","network":true}]"#,
            GrantError::ToolSubGrant,
        ),
        (
            r#"[{"kind":"tool","tool":"git","modify":true}]"#,
            GrantError::ToolSubGrant,
        ),
        (
            r#"[{"kind":"tool","tool":"git"},{"kind":"tool","tool":"git"}]"#,
            GrantError::Duplicate,
        ),
        (
            r#"[{"kind":"resources"},{"kind":"resources","open":true}]"#,
            GrantError::Duplicate,
        ),
        (r#"[{"kind":"profile","read":[]}]"#, GrantError::Malformed),
    ] {
        let approved = ApprovedPermissions::from_json(bad).unwrap();
        assert_eq!(Grants::derive(&approved), Err(error), "{bad}");
    }
    // A raw process, a shell and the file system stay never grantable;
    // `projectContext` is a record of its own now.
    for kind in ["process", "exec", "shell", "filesystem", "workflows", "ui"] {
        assert!(matches!(
            ApprovedPermissions::from_json(&format!(r#"[{{"kind":"{kind}"}}]"#)),
            Err(GrantError::NeverGrantable(_))
        ));
    }
    let g =
        Grants::derive(&ApprovedPermissions::from_json(r#"[{"kind":"projectContext"}]"#).unwrap())
            .unwrap();
    assert!(g.tool(Tool::Git).is_none() && g.resources().is_none());
}

#[test]
fn none_of_them_runs_inside_a_state_call() {
    let (_, s, _) = session(
        r#"[{"kind":"resources","paths":true,"open":true},{"kind":"openUrlFromParams"},
            {"kind":"projectContext","select":true},{"kind":"profile","read":["profile"]},
            {"kind":"tool","tool":"git","network":true},
            {"kind":"tool","tool":"docker","modify":true}]"#,
    );
    for request in every_op() {
        assert_eq!(
            s.authorize_state(&request).denial(),
            Some(DenyReason::NotInStateCall),
            "{}",
            request.op()
        );
    }
}

#[test]
fn a_second_project_switch_within_500_ms_is_throttled() {
    // X6: 2 per second without a burst.
    let clock = Arc::new(ManualClock::default());
    let broker = Broker::new(
        Arc::new(MemoryAudit::new(100)),
        clock.clone(),
        RateLimits::default(),
    );
    let plugin = PluginId::new("deckpilot.projects").unwrap();
    broker
        .set_grants(
            &plugin,
            &ApprovedPermissions::from_json(r#"[{"kind":"projectContext","select":true}]"#)
                .unwrap(),
        )
        .unwrap();
    let s = broker.session(plugin);
    assert!(s.authorize_in(&select(), Some(&user())).is_allowed());
    clock.advance(std::time::Duration::from_millis(400));
    assert_eq!(denial(&s, &cycle(), &user()), Some(DenyReason::RateLimited));
    clock.advance(std::time::Duration::from_millis(100));
    assert!(s.authorize_in(&cycle(), Some(&user())).is_allowed());
}

/// A denied `tool.run` is audited with its tool and subcommand (I3 of the
/// review), never with another argument; a subcommand that is not a plain
/// word is `?`.
#[test]
fn tool_run_denials_name_the_tool_and_subcommand_only() {
    let (_, s, audit) = session(r#"[{"kind":"tool","tool":"git"}]"#);
    let cases: [(Tool, &[&str], &str); 5] = [
        (
            Tool::Git,
            &["ls-remote", "--", "origin", "secret-ref"],
            "git ls-remote",
        ),
        (
            Tool::Docker,
            &["compose", "up", "--detach", "secret-service"],
            "docker compose up",
        ),
        (
            Tool::Git,
            &["-c", "core.sshCommand=secret", "status"],
            "git ?",
        ),
        (Tool::Docker, &["SECRET$(id)", "x"], "docker ?"),
        (
            Tool::Docker,
            &["ps", "--filter", "name=secret"],
            "docker ps",
        ),
    ];
    for (tool, args, _) in &cases {
        assert!(denial(&s, &run(*tool, args), &user()).is_some(), "{args:?}");
    }
    let targets: Vec<String> = audit
        .records()
        .iter()
        .filter(|r| r.op == "tool.run")
        .filter_map(|r| r.target.clone())
        .collect();
    let expected: Vec<&str> = cases.iter().map(|(_, _, t)| *t).collect();
    assert_eq!(targets, expected);
    assert!(
        targets
            .iter()
            .all(|t| !t.contains("secret") && !t.contains("SECRET"))
    );
}

#[test]
fn url_open_param_allows_one_runs_pages_then_one_per_2_s() {
    let clock = Arc::new(ManualClock::default());
    let broker = Broker::new(
        Arc::new(MemoryAudit::new(100)),
        clock.clone(),
        RateLimits::default(),
    );
    let plugin = PluginId::new("deckpilot.workspace").unwrap();
    broker
        .set_grants(
            &plugin,
            &ApprovedPermissions::from_json(r#"[{"kind":"openUrlFromParams"}]"#).unwrap(),
        )
        .unwrap();
    let s = broker.session(plugin);
    for _ in 0..10 {
        assert!(s.authorize_in(&open_param(), Some(&user())).is_allowed());
    }
    assert_eq!(
        denial(&s, &open_param(), &user()),
        Some(DenyReason::RateLimited)
    );
    clock.advance(std::time::Duration::from_secs(2));
    assert!(s.authorize_in(&open_param(), Some(&user())).is_allowed());
}
