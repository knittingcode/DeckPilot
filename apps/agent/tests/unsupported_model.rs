//! A device whose model no driver registered, next to a Stream Deck MK.2,
//! with a stored layout for a model the agent does not know: the unknown
//! device is listed without a descriptor or a layout and gets nothing drawn
//! or run; the MK.2 and its layout work as before.

mod support;

use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, LayoutStatus, ParameterValue,
    Profile, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{
    ConnectionState, ControlId, ControlInput, DeviceId, DeviceModel, DeviceModelId, models,
};
use deckpilot_ipc::{AgentEvent, AgentStatus, Request};
use deckpilot_modules::general;
use support::{TIMEOUT, TempDir, TestAgent};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

/// An MK.2 in all but its id and name: not in the built-in catalog, and the
/// simulated backend contributes no models.
fn unknown_model() -> DeviceModel {
    DeviceModel {
        id: DeviceModelId::new("acme.unknown.u1"),
        name: "Unknown U1".into(),
        vendor: "Acme".into(),
        ..models::stream_deck_mk2()
    }
}

/// Key 0 logs "pressed" on the MK.2; the same binding in a layout for the
/// unknown model.
fn profile() -> Profile {
    let mut profile = Profile::new("Work");
    let action = support::registry()
        .create_instance(
            "Pressed",
            general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text("pressed".into())),
        )
        .unwrap();
    let target = BindingTarget::action(action.id);
    profile.actions.push(action);
    for model in [models::stream_deck_mk2().id, unknown_model().id] {
        let mut layout = DeviceLayout::new("Main", model);
        layout.pages[0]
            .bindings
            .push(ControlBinding::new(key(0), Trigger::Press, target.clone()));
        profile.layouts.push(layout);
    }
    profile
}

async fn wait_status(agent: &TestAgent, predicate: impl Fn(&AgentStatus) -> bool) -> AgentStatus {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let status = agent.status().await;
        if predicate(&status) {
            return status;
        }
        assert!(Instant::now() < deadline, "not reached: {status:#?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn connected(status: &AgentStatus, device: &DeviceId) -> bool {
    status
        .devices
        .iter()
        .any(|d| d.id == *device && d.state == ConnectionState::Connected)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_model_is_listed_but_never_driven() {
    let dir = TempDir::new("deckpilot-unknown-model");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let mk2 = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let unknown = mock.plug(&unknown_model(), "UNKNOWN-1");
    let agent = TestAgent::start_unlocked(&paths, &key_material, &mock, None).await;
    let status = wait_status(&agent, |s| {
        connected(s, &mk2) && connected(s, &unknown) && !mock.frames(&mk2).is_empty()
    })
    .await;

    let info = |device: &DeviceId| status.devices.iter().find(|d| d.id == *device).unwrap();
    assert!(matches!(info(&mk2).layout, LayoutStatus::Compatible { .. }));
    assert!(info(&mk2).descriptor.is_some());
    assert_eq!(info(&unknown).layout, LayoutStatus::UnknownModel);
    assert_eq!(info(&unknown).descriptor, None);
    // The runtime still opens the stored layout's home page for the id
    // (`page` is "Home"), as before the driver registry; nothing is
    // drawn or run from it.

    // Its input runs nothing; the MK.2's does.
    let mut events = agent.events().await;
    for input in [ControlInput::Press, ControlInput::Release] {
        let _ = agent
            .request(Request::SimulateInput {
                device: unknown.clone(),
                control: key(0),
                input,
            })
            .await;
    }
    for input in [ControlInput::Press, ControlInput::Release] {
        agent
            .request(Request::SimulateInput {
                device: mk2.clone(),
                control: key(0),
                input,
            })
            .await
            .unwrap();
    }
    let completed = loop {
        let event = tokio::time::timeout(TIMEOUT, events.next_event())
            .await
            .expect("an event in time")
            .unwrap();
        if let AgentEvent::ActionCompleted { name, .. } = event {
            break name;
        }
    };
    assert_eq!(completed, "Pressed");
    // Exactly one run: the unknown device's press was dropped, not queued.
    let more = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if let Ok(AgentEvent::ActionCompleted { .. }) = events.next_event().await {
                return;
            }
        }
    })
    .await;
    assert!(more.is_err(), "the unknown device ran an action");

    // Nothing is drawn on it, and brightness leaves it alone.
    agent
        .request(Request::SetBrightness { percent: 30 })
        .await
        .unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while mock.brightness(&mk2) != Some(30) {
        assert!(
            Instant::now() < deadline,
            "the MK.2 did not get the brightness"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        mock.frames(&unknown).is_empty(),
        "images for an unknown model"
    );
    assert_ne!(mock.brightness(&unknown), Some(30));
    let status = agent.status().await;
    assert!(connected(&status, &mk2) && connected(&status, &unknown));
    agent.shutdown().await;
}
