//! `url.open` (design §5.2 `openUrl`): exact granted https origins only,
//! never a file, custom scheme, IP literal or local name, rate limited,
//! and audited by origin without the path or query.

use std::sync::Arc;
use std::time::Duration;

use deckpilot_plugin_abi::capability::{HttpFetch, NoArgs, OpenUrl, RunCause, RunTrigger};
use deckpilot_plugin_abi::limits::MAX_OPEN_URL_LEN;
use deckpilot_plugin_abi::{CallErrorCode, CapabilityRequest, HttpMethod};
use deckpilot_plugin_broker::*;

const GITHUB: &str = r#"[{"kind":"openUrl","origin":"https://github.com"}]"#;

struct Fixture {
    broker: Broker,
    audit: Arc<MemoryAudit>,
    clock: ManualClock,
    plugin: PluginId,
}

fn fixture(perms: &str) -> Fixture {
    let audit = Arc::new(MemoryAudit::new(10_000));
    let clock = ManualClock::default();
    let broker = Broker::new(
        audit.clone(),
        Arc::new(clock.clone()),
        RateLimits::default(),
    );
    let plugin = PluginId::new("acme.github").unwrap();
    broker
        .set_grants(&plugin, &ApprovedPermissions::from_json(perms).unwrap())
        .unwrap();
    Fixture {
        broker,
        audit,
        clock,
        plugin,
    }
}

/// `url.open` from an action started by a key press: the runs it is
/// allowed in.
trait ByKey {
    fn by_key(&self, request: &CapabilityRequest) -> Decision;
}

impl ByKey for Session {
    fn by_key(&self, request: &CapabilityRequest) -> Decision {
        self.authorize_in(request, Some(&RunContext::direct(RunTrigger::Key)))
    }
}

fn open(url: &str) -> CapabilityRequest {
    CapabilityRequest::OpenUrl(OpenUrl { url: url.into() })
}

#[test]
fn a_granted_origin_is_opened_normalized_and_audited_without_its_path() {
    let f = fixture(GITHUB);
    let s = f.broker.session(f.plugin.clone());
    let d = s.by_key(&open(
        "https://GitHub.com:443/acme/app/pull/7?access_token=s3cret#diff",
    ));
    let Decision::Allow(Allowed::OpenUrl { url, origin }) = d else {
        panic!("{d:?}")
    };
    assert_eq!(
        url.as_str(),
        "https://github.com/acme/app/pull/7?access_token=s3cret#diff"
    );
    assert_eq!(origin.to_string(), "https://github.com");
    let records = f.audit.records();
    let last = records.last().unwrap();
    assert_eq!(last.op, "url.open");
    assert_eq!(last.target.as_deref(), Some("https://github.com"));
    assert_eq!(last.decision, AuditDecision::Allowed);
    let all = format!("{records:?}");
    assert!(!all.contains("s3cret") && !all.contains("pull"), "{all}");
}

#[test]
fn only_https_urls_of_a_granted_origin_are_opened() {
    let f = fixture(GITHUB);
    let s = f.broker.session(f.plugin.clone());
    let cases = [
        // Other hosts, look-alikes, subdomains and ports.
        ("https://gist.github.com/", DenyReason::OriginNotGranted),
        ("https://github.com.evil.net/", DenyReason::OriginNotGranted),
        ("https://evilgithub.com/", DenyReason::OriginNotGranted),
        ("https://github.co/", DenyReason::OriginNotGranted),
        ("https://github.com:8443/", DenyReason::OriginNotGranted),
        (
            "https://evil.net/?next=https://github.com/",
            DenyReason::OriginNotGranted,
        ),
        // Schemes that open files or start programs.
        (
            "http://github.com/",
            DenyReason::Url(OriginError::SchemeNotAllowed),
        ),
        (
            "file:///etc/passwd",
            DenyReason::Url(OriginError::SchemeNotAllowed),
        ),
        (
            "file://github.com/etc/passwd",
            DenyReason::Url(OriginError::SchemeNotAllowed),
        ),
        (
            "javascript:alert(1)",
            DenyReason::Url(OriginError::SchemeNotAllowed),
        ),
        (
            "data:text/html,<script>",
            DenyReason::Url(OriginError::SchemeNotAllowed),
        ),
        (
            "x-github-client://openRepo/https://github.com/a/b",
            DenyReason::Url(OriginError::SchemeNotAllowed),
        ),
        (
            "vscode://file/etc/passwd",
            DenyReason::Url(OriginError::SchemeNotAllowed),
        ),
        (
            "mailto:a@github.com",
            DenyReason::Url(OriginError::SchemeNotAllowed),
        ),
        (
            "smb://github.com/share",
            DenyReason::Url(OriginError::SchemeNotAllowed),
        ),
        (
            "//github.com/",
            DenyReason::Url(OriginError::SchemeNotAllowed),
        ),
        (
            "github.com/acme",
            DenyReason::Url(OriginError::SchemeNotAllowed),
        ),
        // Userinfo: the real host is another one, or a phishing prefix.
        (
            "https://github.com@evil.net/",
            DenyReason::Url(OriginError::UserinfoNotAllowed),
        ),
        (
            "https://user:pw@github.com/",
            DenyReason::Url(OriginError::UserinfoNotAllowed),
        ),
        // Private and local hosts, whatever the grant.
        (
            "https://127.0.0.1/",
            DenyReason::Url(OriginError::IpLiteralNotAllowed),
        ),
        (
            "https://[::1]/",
            DenyReason::Url(OriginError::IpLiteralNotAllowed),
        ),
        (
            "https://192.168.1.1/",
            DenyReason::Url(OriginError::IpLiteralNotAllowed),
        ),
        (
            "https://localhost/",
            DenyReason::Url(OriginError::InvalidHost),
        ),
        (
            "https://router.local/",
            DenyReason::Url(OriginError::InvalidHost),
        ),
        // Characters the WHATWG parser would silently drop or reinterpret.
        (
            "https://github.com\\@evil.net/",
            DenyReason::Url(OriginError::IllegalCharacters),
        ),
        (
            "https://git\thub.com/",
            DenyReason::Url(OriginError::IllegalCharacters),
        ),
        (
            "https://github.com/\n",
            DenyReason::Url(OriginError::IllegalCharacters),
        ),
        (
            " https://github.com/",
            DenyReason::Url(OriginError::IllegalCharacters),
        ),
    ];
    for (url, why) in cases {
        let d = s.by_key(&open(url));
        assert_eq!(d.denial(), Some(why), "{url:?}");
    }
    // Too long, before and after normalization.
    let long = format!("https://github.com/{}", "a".repeat(MAX_OPEN_URL_LEN));
    assert_eq!(s.by_key(&open(&long)).denial(), Some(DenyReason::TooLarge));
    let expands = format!("https://github.com/{}", "é".repeat(MAX_OPEN_URL_LEN / 4));
    assert!(expands.len() <= MAX_OPEN_URL_LEN);
    assert_eq!(
        s.by_key(&open(&expands)).denial(),
        Some(DenyReason::TooLarge)
    );
    // No audit record carries a path, query or a refused URL's text.
    let all = format!("{:?}", f.audit.records());
    for fragment in ["passwd", "alert", "evil.net/", "share", "openRepo"] {
        assert!(
            !all.contains(fragment),
            "an audit record holds `{fragment}`"
        );
    }
}

#[test]
fn open_url_and_network_grants_do_not_imply_each_other() {
    let f = fixture(GITHUB);
    let s = f.broker.session(f.plugin.clone());
    let fetch = CapabilityRequest::HttpFetch(HttpFetch {
        method: HttpMethod::Get,
        url: "https://github.com/".into(),
        headers: vec![],
        body: None,
        auth: None,
    });
    assert_eq!(
        s.authorize(&fetch).denial(),
        Some(DenyReason::OriginNotGranted)
    );
    let f = fixture(r#"[{"kind":"network","origin":"https://github.com","methods":["GET"]}]"#);
    let s = f.broker.session(f.plugin.clone());
    assert!(s.authorize(&fetch).is_allowed());
    let d = s.by_key(&open("https://github.com/"));
    assert_eq!(d.denial(), Some(DenyReason::OriginNotGranted));
    assert_eq!(
        d.denial().unwrap().call_error_code(),
        CallErrorCode::NotPermitted
    );
}

#[test]
fn opening_is_rate_limited_short_and_long() {
    let f = fixture(GITHUB);
    let s = f.broker.session(f.plugin.clone());
    let url = "https://github.com/acme/app";
    assert!(s.by_key(&open(url)).is_allowed());
    // 1 per 2 s.
    let d = s.by_key(&open(url));
    assert_eq!(d.denial(), Some(DenyReason::RateLimited));
    assert_eq!(
        d.denial().unwrap().call_error_code(),
        CallErrorCode::Throttled
    );
    f.clock.advance(Duration::from_millis(1_000));
    assert_eq!(s.by_key(&open(url)).denial(), Some(DenyReason::RateLimited));
    f.clock.advance(Duration::from_millis(1_000));
    assert!(s.by_key(&open(url)).is_allowed());
    // At most 20 in 10 minutes, even at the short rate.
    let mut opened = 2;
    for _ in 0..40 {
        f.clock.advance(Duration::from_secs(2));
        if s.by_key(&open(url)).is_allowed() {
            opened += 1;
        }
    }
    // 82 s passed: 20 at once plus about 2.7 refilled.
    assert!((20..=23).contains(&opened), "{opened}");
    // A denial for another reason costs no token.
    let g = fixture(GITHUB);
    let t = g.broker.session(g.plugin.clone());
    for _ in 0..5 {
        assert_eq!(
            t.by_key(&open("https://gitlab.com/")).denial(),
            Some(DenyReason::OriginNotGranted)
        );
    }
    assert!(t.by_key(&open(url)).is_allowed());
}

#[test]
fn a_revoked_or_removed_grant_applies_on_the_next_call() {
    let f = fixture(
        r#"[{"kind":"openUrl","origin":"https://github.com"},
            {"kind":"openUrl","origin":"https://gitlab.com"}]"#,
    );
    let s = f.broker.session(f.plugin.clone());
    assert!(s.by_key(&open("https://github.com/")).is_allowed());
    f.broker.revoke_open_url(
        &f.plugin,
        &Origin::parse_grant("https://github.com").unwrap(),
    );
    f.clock.advance(Duration::from_secs(3));
    assert_eq!(
        s.by_key(&open("https://github.com/")).denial(),
        Some(DenyReason::OriginNotGranted)
    );
    assert!(s.by_key(&open("https://gitlab.com/")).is_allowed());
    f.broker.remove_plugin(&f.plugin);
    f.clock.advance(Duration::from_secs(3));
    assert_eq!(
        s.by_key(&open("https://gitlab.com/")).denial(),
        Some(DenyReason::UnknownPlugin)
    );
    // Another plugin's session never uses this plugin's grant.
    let other = f.broker.session(PluginId::new("acme.other").unwrap());
    assert_eq!(
        other.by_key(&open("https://gitlab.com/")).denial(),
        Some(DenyReason::UnknownPlugin)
    );
}

/// G8: a page opens only in a run the user started (a key, a gesture, the
/// app, the plugin's page, a workflow the user started). A scheduled,
/// event, startup or work-session run, or one nobody described, is
/// denied before the rate limits, and audited as denied.
#[test]
fn pages_open_only_in_runs_the_user_started() {
    let f = fixture(GITHUB);
    let s = f.broker.session(f.plugin.clone());
    let url = "https://github.com/acme/app";
    for context in [
        RunContext::direct(RunTrigger::Key),
        RunContext::direct(RunTrigger::Gesture),
        RunContext::direct(RunTrigger::App),
        RunContext::direct(RunTrigger::Test),
        RunContext::workflow(RunCause::User, None),
    ] {
        f.clock.advance(Duration::from_secs(3));
        let d = s.authorize_in(&open(url), Some(&context));
        assert!(d.is_allowed(), "{context:?}: {d:?}");
    }
    let denied_before = f.audit.records().len();
    for context in [
        Some(RunContext::workflow(RunCause::Schedule, None)),
        Some(RunContext::workflow(RunCause::Event, None)),
        Some(RunContext::workflow(RunCause::Startup, None)),
        Some(RunContext::workflow(RunCause::WorkSession, None)),
        Some(RunContext::workflow(RunCause::Other, None)),
        Some(RunContext::unknown()),
        None,
    ] {
        f.clock.advance(Duration::from_secs(3));
        let d = s.authorize_in(&open(url), context.as_ref());
        assert_eq!(
            d.denial(),
            Some(DenyReason::NotUserInitiated),
            "{context:?}"
        );
        assert_eq!(
            DenyReason::NotUserInitiated.call_error_code(),
            CallErrorCode::NotPermitted
        );
    }
    // `authorize` without a context is the same as none.
    f.clock.advance(Duration::from_secs(3));
    assert_eq!(
        s.authorize(&open(url)).denial(),
        Some(DenyReason::NotUserInitiated)
    );
    let records = f.audit.records();
    assert!(
        records[denied_before..].iter().all(|r| r.op == "url.open"
            && r.decision == AuditDecision::Denied(DenyReason::NotUserInitiated)
            && r.target.as_deref() == Some("https://github.com")),
        "{records:?}"
    );
    // A refused call took no rate token: the next allowed one at once.
    let d = s.by_key(&open(url));
    assert!(d.is_allowed(), "{d:?}");
    // An origin that is not granted stays "not granted" in any run.
    assert_eq!(
        s.authorize_in(
            &open("https://gitlab.com/"),
            Some(&RunContext::workflow(RunCause::Schedule, None))
        )
        .denial(),
        Some(DenyReason::OriginNotGranted)
    );
    assert!(
        DenyReason::NotUserInitiated
            .to_string()
            .contains("a key, a gesture or the app")
    );
}

/// `run.context` answers exactly the context the agent handed over (the
/// invocation's own run, within the ABI's bounds), `unknown` without one,
/// for any plugin with grants.
#[test]
fn run_context_answers_the_invocations_own_run() {
    let f = fixture("[]");
    let s = f.broker.session(f.plugin.clone());
    let request = CapabilityRequest::RunContext(NoArgs {});
    let run = deckpilot_plugin_abi::capability::RunSummary {
        workflow: Some("w".repeat(500)),
        failed: 1,
        failed_nodes: vec!["deploy".into()],
        ..Default::default()
    };
    let context = RunContext::workflow(RunCause::Schedule, Some(run));
    let Decision::Allow(Allowed::RunContext(got)) = s.authorize_in(&request, Some(&context)) else {
        panic!()
    };
    assert_eq!(got, context.clone().bounded());
    assert_eq!(got.run.unwrap().workflow.unwrap().chars().count(), 128);
    let Decision::Allow(Allowed::RunContext(got)) = s.authorize(&request) else {
        panic!()
    };
    assert_eq!(got, RunContext::unknown());
    // A plugin without grants gets nothing.
    let stranger = f.broker.session(PluginId::new("acme.other").unwrap());
    assert_eq!(
        stranger.authorize_in(&request, Some(&context)).denial(),
        Some(DenyReason::UnknownPlugin)
    );
}
