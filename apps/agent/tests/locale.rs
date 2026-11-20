//! The app's language on the agent (protocol 13, `setLocale`): the texts
//! the agent produces itself on keys follow it, and only the keys showing
//! them are drawn again; titles the user wrote never change. A client of
//! protocol 12 cannot set it, and everything stays English.

mod support;

use std::time::Duration;

use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, FaceStyle, KeyFace, Profile,
    Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, DeviceId, Frame, models};
use deckpilot_ipc::{AgentClient, ClientError, ClientKind, Request, Response};
use deckpilot_modules::{deck, general};

use support::{TIMEOUT, TempDir, TestAgent};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn press(index: u16, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(key(index), Trigger::Press, target)
}

fn action(
    profile: &mut Profile,
    name: &str,
    action_type: deckpilot_core::ActionTypeId,
    parameters: &ActionParameters,
) -> BindingTarget {
    let instance = support::registry()
        .create_instance(name, action_type, parameters)
        .unwrap();
    let target = BindingTarget::action(instance.id);
    profile.actions.push(instance);
    target
}

/// Key 0: *Next page* under its default name; key 1: the same action the
/// user renamed; key 2: a Back key without a title; key 3: a Back key the
/// user titled "Back"; key 4: a log action.
fn profile() -> Profile {
    let mut profile = Profile::new("Work");
    let next = action(
        &mut profile,
        "Next page",
        deck::next_page_id(),
        &ActionParameters::new(),
    );
    let renamed = action(
        &mut profile,
        "Forward",
        deck::next_page_id(),
        &ActionParameters::new(),
    );
    let log = action(
        &mut profile,
        "Log it",
        general::log_id(),
        &ActionParameters::new().with(
            "message",
            deckpilot_core::ParameterValue::Text("hello".into()),
        ),
    );
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let page = &mut layout.pages[0];
    page.bindings.extend([
        press(0, next),
        press(1, renamed),
        press(2, BindingTarget::Back),
        ControlBinding {
            label: Some("Back".into()),
            ..press(3, BindingTarget::Back)
        },
        press(4, log),
    ]);
    profile.layouts.push(layout);
    profile
}

fn image(face: &KeyFace) -> Frame {
    let spec = models::stream_deck_mk2()
        .capabilities
        .keys
        .and_then(|keys| keys.image)
        .unwrap();
    deckpilot_agent::render::render_with(
        face,
        spec.width,
        spec.height,
        &deckpilot_agent::render::Icons::default(),
    )
}

fn next_page(title: &str) -> Frame {
    let icon = support::registry()
        .definition(&deck::next_page_id())
        .and_then(|definition| definition.node.icon.clone());
    image(&KeyFace::new(FaceStyle::Action, title).with_symbol(icon))
}

fn back(title: &str) -> Frame {
    image(
        &KeyFace::new(FaceStyle::Back, title)
            .with_symbol(Some(deckpilot_core::device_runtime::symbols::BACK)),
    )
}

async fn connected(agent: &TestAgent, device: &DeviceId) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let status = agent.status().await;
        if status
            .devices
            .iter()
            .any(|d| d.id == *device && d.state == ConnectionState::Connected && d.page.is_some())
        {
            return;
        }
        assert!(tokio::time::Instant::now() <= deadline, "not connected");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn set_locale(client: &mut AgentClient, locale: &str) {
    assert!(client.set_locale(locale).await.unwrap(), "sent");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_language_change_redraws_only_the_keys_with_deckpilots_texts() {
    let dir = TempDir::new("deckpilot-locale-keys");
    let (paths, db_key) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-L");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &device).await;
    assert!(mock.wait_for(&device, TIMEOUT, |frames, _| {
        frames.get(&key(0)) == Some(&next_page("Next page"))
            && frames.get(&key(2)) == Some(&back("Back"))
    }));
    assert_eq!(
        agent.status().await.locale,
        None,
        "English until an app says"
    );
    let before = mock.frames(&device);
    assert_eq!(before.get(&key(3)), Some(&back("Back")));
    let writes = mock.image_writes(&device);

    let mut client = agent.client().await;
    set_locale(&mut client, "pl-PL").await;
    assert!(
        mock.wait_for(&device, TIMEOUT, |frames, _| {
            frames.get(&key(0)) == Some(&next_page("Następna strona"))
                && frames.get(&key(2)) == Some(&back("Wstecz"))
        }),
        "the default name and the Back key in Polish"
    );
    let after = mock.frames(&device);
    // The renamed action, the titled Back key and the log key stay.
    for index in [1, 3, 4] {
        assert_eq!(
            after.get(&key(index)),
            before.get(&key(index)),
            "key {index}"
        );
    }
    assert_eq!(
        after.get(&key(3)),
        Some(&back("Back")),
        "the user's own title"
    );
    assert_eq!(mock.image_writes(&device) - writes, 2, "only the two keys");
    assert_eq!(agent.status().await.locale.as_deref(), Some("pl-PL"));

    // The same language again changes nothing.
    set_locale(&mut client, "pl-PL").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(mock.image_writes(&device) - writes, 2);

    // Back to English: as before.
    set_locale(&mut client, "en").await;
    assert!(mock.wait_for(&device, TIMEOUT, |frames, _| *frames == before));
    assert_eq!(mock.image_writes(&device) - writes, 4);

    // Not a tag: refused, nothing changes.
    let refused = client
        .request(Request::SetLocale {
            locale: "../pl".into(),
        })
        .await;
    assert!(matches!(refused, Err(ClientError::Agent(_))), "{refused:?}");
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_12_app_never_sets_the_language() {
    let dir = TempDir::new("deckpilot-locale-p12");
    let (paths, db_key) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-M");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &device).await;
    assert!(mock.wait_for(&device, TIMEOUT, |frames, _| {
        frames.get(&key(2)) == Some(&back("Back"))
    }));
    let mut old = AgentClient::connect_with_protocol(&paths, ClientKind::Desktop, 12)
        .await
        .unwrap();
    assert!(!old.set_locale("pl").await.unwrap(), "not sent");
    assert!(matches!(
        old.request(Request::SetLocale {
            locale: "pl".into()
        })
        .await,
        Err(ClientError::Unsupported { needs: 13, .. })
    ));
    // Everything else works as before; the keys stay English.
    assert!(matches!(
        old.request(Request::GetStatus).await,
        Ok(Response::Status { status }) if status.locale.is_none()
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(mock.frames(&device).get(&key(2)), Some(&back("Back")));
    agent.shutdown().await;
}
