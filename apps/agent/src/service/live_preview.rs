//! The editor's live preview (IPC protocol 16, `deckpilot_ipc::live_preview`,
//! `docs/editor-navigation-and-simulation.md`).
//!
//! A session belongs to one connection (its `viewer` id, as for
//! `watchKeyStates`) and previews one layout of the active profile on one
//! device: the runtime's working layout of that device
//! ([`deckpilot_core::Runtime::set_working_layout`]). Nothing is saved:
//! applying a revision or navigating only redraws the device (the face
//! diff of `Agent::render` writes the keys that changed) and publishes the
//! status. When the session ends (asked, its connection closed, locked),
//! the device shows the saved layout again.
//!
//! Navigation the device makes by itself (a key, Auto Exit, a reload) is
//! sent to the owner after every command ([`Agent::sync_live_previews`]),
//! each state with a sequence number that only grows, so an answer and an
//! event that cross each other can be ordered by the client.

use std::collections::BTreeMap;

use deckpilot_core::{DeviceLayout, EditorTarget, LayoutId, NavigationError, ProfileId};
use deckpilot_device::DeviceId;
use deckpilot_ipc::live_preview::{
    DeviceNavigation, EditorNavigation, EndReason, LivePreviewEvent, MockDeviceInspection,
    MockFrame, RevisionGate, frame_hash, reason,
};
use deckpilot_ipc::{ErrorCode, IpcError, Request, Response, SecurityState};
use tracing::{debug, info};

use super::Agent;

/// The open sessions.
#[derive(Debug, Default)]
pub(crate) struct LivePreviews {
    sessions: BTreeMap<u64, Session>,
    next: u64,
    /// Numbers the navigation states sent, across sessions.
    seq: u64,
}

#[derive(Debug)]
struct Session {
    owner: u64,
    device: DeviceId,
    profile: ProfileId,
    layout: LayoutId,
    gate: RevisionGate,
    /// What the owner last heard (sequence numbers aside).
    reported: Option<DeviceNavigation>,
}

impl LivePreviews {
    fn of(&self, owner: u64, session: u64) -> Result<&Session, IpcError> {
        self.sessions
            .get(&session)
            .filter(|s| s.owner == owner)
            .ok_or_else(|| {
                refused(
                    ErrorCode::NotFound,
                    reason::UNKNOWN_SESSION,
                    "no such live preview session",
                )
            })
    }

    /// Whether `device` has a session.
    pub(crate) fn previews(&self, device: &DeviceId) -> bool {
        self.sessions.values().any(|s| s.device == *device)
    }
}

fn refused(code: ErrorCode, why: &str, message: impl Into<String>) -> IpcError {
    IpcError::new(code, message).with_reason(why)
}

fn navigation_error(error: &NavigationError) -> IpcError {
    match error {
        NavigationError::UnknownPage => {
            refused(ErrorCode::NotFound, reason::UNKNOWN_PAGE, error.to_string())
        }
        NavigationError::UnknownLayout => {
            refused(ErrorCode::NotFound, reason::WRONG_MODEL, error.to_string())
        }
        NavigationError::NotConnected
        | NavigationError::NoProfile
        | NavigationError::NoPreviousLayout => IpcError::new(ErrorCode::Failed, error.to_string()),
    }
}

fn target(navigation: EditorNavigation) -> EditorTarget {
    match navigation {
        EditorNavigation::Show { layout, stack } => EditorTarget::Show { layout, stack },
        EditorNavigation::LayoutBack => EditorTarget::LayoutBack,
    }
}

/// What saving the layout would store for it, where that changes what the
/// device shows (the desktop's `save_layout`): labels trimmed (empty: the
/// default), default appearances not kept.
fn normalized(mut layout: DeviceLayout) -> DeviceLayout {
    for binding in layout.pages.iter_mut().flat_map(|page| &mut page.bindings) {
        binding.label = binding
            .label
            .take()
            .map(|label| label.trim().to_owned())
            .filter(|label| !label.is_empty());
        binding.appearance = binding
            .appearance
            .take()
            .and_then(deckpilot_core::KeyAppearance::normalized);
    }
    layout
}

impl Agent {
    /// A live preview request of connection `viewer`.
    pub(super) fn live_preview(
        &mut self,
        viewer: u64,
        request: Request,
    ) -> Result<Response, IpcError> {
        if self.security != SecurityState::Unlocked {
            return Err(super::locked());
        }
        match request {
            Request::BeginLivePreview {
                device,
                profile,
                layout,
                navigation,
            } => self.begin_live_preview(viewer, device, profile, layout, navigation),
            Request::ApplyLivePreview {
                session,
                revision,
                part,
                last,
            } => self.apply_live_preview(viewer, session, revision, *part, last),
            Request::NavigateLivePreview {
                session,
                navigation,
            } => self.navigate_live_preview(viewer, session, navigation),
            Request::EndLivePreview { session } => {
                self.live_previews.of(viewer, session)?;
                self.end_live_preview(session, None);
                Ok(Response::Ok)
            }
            _ => Err(IpcError::new(
                ErrorCode::InvalidRequest,
                "not a live preview request",
            )),
        }
    }

    fn begin_live_preview(
        &mut self,
        viewer: u64,
        device: DeviceId,
        profile: ProfileId,
        layout: LayoutId,
        navigation: Option<EditorNavigation>,
    ) -> Result<Response, IpcError> {
        let Some(active) = self.runtime.profile() else {
            return Err(refused(
                ErrorCode::Failed,
                reason::INACTIVE_PROFILE,
                "no profile is active",
            ));
        };
        if active.id != profile {
            return Err(refused(
                ErrorCode::Failed,
                reason::INACTIVE_PROFILE,
                "the profile is not the active one",
            ));
        }
        let Some(saved) = active
            .layout(layout)
            .filter(|saved| saved.model == device.model)
            .cloned()
        else {
            return Err(refused(
                ErrorCode::NotFound,
                reason::WRONG_MODEL,
                "the layout is not one of the profile's for this device",
            ));
        };
        if self
            .live_previews
            .sessions
            .values()
            .any(|s| s.device == device && s.owner != viewer)
        {
            return Err(refused(
                ErrorCode::Failed,
                reason::DEVICE_BUSY,
                "another window previews on this device",
            ));
        }
        // One session per connection: a new one replaces it.
        let previous: Vec<u64> = self
            .live_previews
            .sessions
            .iter()
            .filter(|(_, s)| s.owner == viewer)
            .map(|(id, _)| *id)
            .collect();
        for id in previous {
            let same_device = self.live_previews.sessions[&id].device == device;
            self.end_live_preview(id, Some(EndReason::Replaced));
            if same_device {
                debug!(%device, "live preview replaced on the same device");
            }
        }
        self.live_previews.next += 1;
        let id = self.live_previews.next;
        self.runtime.set_working_layout(&device, profile, saved);
        if let Some(navigation) = navigation {
            // Best effort: the editor's place may not exist yet.
            if let Err(error) = self.runtime.navigate(&device, &target(navigation)) {
                debug!(%error, "the editor's place could not be shown at the start");
            }
        }
        self.after_preview_change(&device);
        let state = self.device_navigation(&device, profile, layout);
        info!(%device, session = id, "live preview started");
        self.live_previews.sessions.insert(
            id,
            Session {
                owner: viewer,
                device,
                profile,
                layout,
                gate: RevisionGate::default(),
                reported: Some(state.clone()),
            },
        );
        Ok(Response::LivePreviewStarted {
            session: id,
            navigation: state,
        })
    }

    fn apply_live_preview(
        &mut self,
        viewer: u64,
        session: u64,
        revision: u64,
        part: deckpilot_ipc::live_preview::LayoutPatch,
        last: bool,
    ) -> Result<Response, IpcError> {
        self.live_previews.of(viewer, session)?;
        let entry = self
            .live_previews
            .sessions
            .get_mut(&session)
            .expect("checked above");
        let parts = entry.gate.accept(revision, part, last).map_err(|why| {
            let code = if why == reason::STALE_REVISION {
                ErrorCode::InvalidRequest
            } else {
                ErrorCode::Failed
            };
            refused(code, why, format!("revision {revision} was not applied"))
        })?;
        let Some(parts) = parts else {
            return Ok(Response::LivePreviewApplied {
                session,
                revision,
                applied: false,
            });
        };
        let (device, profile, layout) = (entry.device.clone(), entry.profile, entry.layout);
        let base = self
            .runtime
            .working_layout(&device)
            .filter(|working| working.id == layout)
            .or_else(|| self.runtime.profile().and_then(|p| p.layout(layout)))
            .cloned()
            .ok_or_else(|| {
                refused(
                    ErrorCode::NotFound,
                    reason::WRONG_MODEL,
                    "the layout is gone",
                )
            })?;
        let working = deckpilot_ipc::live_preview::apply(&base, &parts).map_err(|message| {
            refused(ErrorCode::InvalidRequest, reason::INVALID_LAYOUT, message)
        })?;
        self.runtime
            .set_working_layout(&device, profile, normalized(working));
        if let Some(entry) = self.live_previews.sessions.get_mut(&session) {
            entry.gate.commit(revision);
        }
        self.after_preview_change(&device);
        Ok(Response::LivePreviewApplied {
            session,
            revision,
            applied: true,
        })
    }

    fn navigate_live_preview(
        &mut self,
        viewer: u64,
        session: u64,
        navigation: EditorNavigation,
    ) -> Result<Response, IpcError> {
        let entry = self.live_previews.of(viewer, session)?;
        let (device, profile, layout) = (entry.device.clone(), entry.profile, entry.layout);
        self.runtime
            .navigate(&device, &target(navigation))
            .map_err(|error| navigation_error(&error))?;
        self.after_preview_change(&device);
        let state = self.device_navigation(&device, profile, layout);
        // Answered, so not sent again as an event.
        if let Some(entry) = self.live_previews.sessions.get_mut(&session) {
            entry.reported = Some(state.clone());
        }
        Ok(Response::LivePreviewNavigated {
            session,
            navigation: state,
        })
    }

    /// Ends `session`: its device shows the saved layout again. With
    /// `reason`, the owner is told.
    pub(super) fn end_live_preview(&mut self, session: u64, reason: Option<EndReason>) {
        let Some(entry) = self.live_previews.sessions.remove(&session) else {
            return;
        };
        if let Some(reason) = reason {
            let _ = self
                .handle
                .live_preview
                .send((entry.owner, LivePreviewEvent::Ended { session, reason }));
        }
        info!(device = %entry.device, session, "live preview ended");
        if self.runtime.clear_working_layout(&entry.device) {
            self.after_preview_change(&entry.device);
        }
    }

    /// The connection `viewer` closed: its sessions end.
    pub(super) fn live_preview_closed(&mut self, viewer: u64) {
        let sessions: Vec<u64> = self
            .live_previews
            .sessions
            .iter()
            .filter(|(_, s)| s.owner == viewer)
            .map(|(id, _)| *id)
            .collect();
        for session in sessions {
            self.end_live_preview(session, None);
        }
    }

    /// Every session ends (locked, stopping).
    pub(super) fn end_all_live_previews(&mut self, reason: EndReason) {
        let sessions: Vec<u64> = self.live_previews.sessions.keys().copied().collect();
        for session in sessions {
            self.end_live_preview(session, Some(reason));
        }
    }

    /// Tells owners where their device went by itself since they last
    /// heard. Called after every command and device signal.
    pub(super) fn sync_live_previews(&mut self) {
        if self.live_previews.sessions.is_empty() {
            return;
        }
        let ids: Vec<u64> = self.live_previews.sessions.keys().copied().collect();
        for id in ids {
            let (device, profile, layout, reported) = {
                let s = &self.live_previews.sessions[&id];
                (s.device.clone(), s.profile, s.layout, s.reported.clone())
            };
            let now = self.navigation_of(&device, profile, layout);
            let changed = reported.as_ref().is_none_or(|before| {
                DeviceNavigation {
                    seq: 0,
                    ..before.clone()
                } != DeviceNavigation {
                    seq: 0,
                    ..now.clone()
                }
            });
            if !changed {
                continue;
            }
            self.live_previews.seq += 1;
            let navigation = DeviceNavigation {
                seq: self.live_previews.seq,
                ..now
            };
            let owner = self.live_previews.sessions[&id].owner;
            if let Some(entry) = self.live_previews.sessions.get_mut(&id) {
                entry.reported = Some(navigation.clone());
            }
            let _ = self.handle.live_preview.send((
                owner,
                LivePreviewEvent::Navigated {
                    session: id,
                    navigation,
                },
            ));
        }
    }

    /// What `device` shows, with a new sequence number.
    fn device_navigation(
        &mut self,
        device: &DeviceId,
        profile: ProfileId,
        layout: LayoutId,
    ) -> DeviceNavigation {
        self.live_previews.seq += 1;
        DeviceNavigation {
            seq: self.live_previews.seq,
            ..self.navigation_of(device, profile, layout)
        }
    }

    fn navigation_of(
        &self,
        device: &DeviceId,
        profile: ProfileId,
        layout: LayoutId,
    ) -> DeviceNavigation {
        let state = self.runtime.navigation_state(device);
        DeviceNavigation {
            in_scope: state.profile == Some(profile) && state.layout == Some(layout),
            profile: state.profile,
            layout: state.layout,
            stack: state.stack,
            connected: state.connected,
            seq: 0,
        }
    }

    /// Redraws `device` (only the keys whose face changed) and tells
    /// clients; nothing is saved.
    fn after_preview_change(&mut self, device: &DeviceId) {
        self.render(device, false);
        self.refresh_states();
        self.publish();
    }

    /// `inspectMockDevice`: a simulated device's frames and navigation.
    pub(super) fn inspect_mock_device(&self, device: &DeviceId) -> Result<Response, IpcError> {
        let mock = self.options.mock.as_ref().ok_or_else(|| {
            IpcError::new(
                ErrorCode::Unsupported,
                "the agent is not using mock devices",
            )
        })?;
        if !self.sessions.contains_key(device) {
            return Err(IpcError::new(
                ErrorCode::NotFound,
                "device is not connected",
            ));
        }
        let frames = mock.frames(device);
        let writes = mock.control_writes(device);
        let state = self.runtime.navigation_state(device);
        let model = self.runtime.catalog().get(&device.model);
        let controls = model.map(|m| m.capabilities.controls()).unwrap_or_default();
        Ok(Response::MockDevice {
            inspection: Box::new(MockDeviceInspection {
                navigation: DeviceNavigation {
                    in_scope: self.live_previews.previews(device)
                        && self.runtime.working_layout(device).map(|l| l.id) == state.layout,
                    profile: state.profile,
                    layout: state.layout,
                    stack: state.stack,
                    connected: state.connected,
                    seq: self.live_previews.seq,
                },
                frames: controls
                    .into_iter()
                    .filter(|control| model.is_some_and(|m| m.capabilities.has_display(*control)))
                    .map(|control| MockFrame {
                        control,
                        hash: frames.get(&control).map(|frame| frame_hash(frame.rgb())),
                        writes: writes.get(&control).copied().unwrap_or(0) as u64,
                    })
                    .collect(),
                writes: mock.image_writes(device) as u64,
            }),
        })
    }
}
