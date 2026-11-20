//! The Device Editor's live preview on the device (IPC protocol 16,
//! `docs/editor-navigation-and-simulation.md`).
//!
//! While the editor shows a layout of the active profile for a connected
//! device, that device shows and runs the editor's unsaved draft: the
//! agent keeps it in memory for that device only, and nothing is saved.
//! One session at a time, on a connection of its own (like the live key
//! states): it ends when the editor ends it, when this app ends, or when
//! the main window reloads or closes; the agent then shows the saved
//! configuration again. When the agent goes away the session is begun
//! again once it is back, and the latest draft and the editor's place are
//! sent again.
//!
//! The editor hands over its whole draft (`live_preview_update`); what
//! goes to the agent is computed here ([`Outbox`]): only the latest draft
//! (drafts that arrive while one is on its way replace each other), as
//! page-level parts of what changed since the agent last applied one, and
//! always before a navigation, so a page that exists only in the draft is
//! on the agent first.
//!
//! The frontend gets [`LIVE_PREVIEW_EVENT`]s tagged with the session's
//! number, so a late event of a replaced session is ignored.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use tokio::sync::mpsc;

use deckpilot_core::{DeviceLayout, LayoutId, ProfileId};
use deckpilot_device::DeviceId;
use deckpilot_ipc::live_preview::{
    DeviceNavigation, EditorNavigation, LayoutPatch, LivePreviewEvent as AgentLiveEvent,
    PART_BUDGET, patch, reason,
};
use deckpilot_ipc::{
    AgentClient, AgentPaths, Capability, ClientError, ClientKind, Request, Response, ServerMessage,
};

use crate::state::AppState;

/// Event with what the live preview session learns ([`LivePreviewEvent`]).
pub const LIVE_PREVIEW_EVENT: &str = "live-preview";

/// How long to wait before connecting again after the agent went away.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
/// How often a session is asked again while the device is still another
/// connection's (the previous session's connection is closing).
const BUSY_RETRIES: u32 = 20;
const BUSY_DELAY: Duration = Duration::from_millis(100);

/// What the editor learns about its session.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LivePreviewUpdate {
    /// The session runs; the device shows `navigation`. `resumed`: after
    /// the agent came back.
    Started {
        navigation: DeviceNavigation,
        resumed: bool,
    },
    /// The device shows this revision of the draft.
    Applied { revision: u64 },
    /// Where the device is: by itself (`own: false`), or as the editor's
    /// navigation went (`own: true`).
    Navigated {
        navigation: DeviceNavigation,
        own: bool,
    },
    /// The agent refused something; `reason` is machine-readable
    /// (`deckpilot_ipc::live_preview::reason`).
    Refused {
        reason: Option<String>,
        message: String,
    },
    /// The session ended on the agent's side (e.g. locked); it is begun
    /// again when possible.
    Ended { reason: String },
    /// The agent is not running or went away; the device shows the saved
    /// configuration.
    Disconnected,
    /// The agent predates the live preview (protocol 16): changes reach
    /// the device when saved.
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LivePreviewEvent {
    /// The session it belongs to, from [`live_preview_begin`].
    pub session: u64,
    #[serde(flatten)]
    pub update: LivePreviewUpdate,
}

/// The running session, if any.
#[derive(Debug, Default)]
pub struct LivePreview {
    inner: Mutex<Running>,
}

#[derive(Debug, Default)]
struct Running {
    number: u64,
    task: Option<tauri::async_runtime::JoinHandle<()>>,
    inbox: Option<mpsc::UnboundedSender<Change>>,
}

impl LivePreview {
    fn lock(&self) -> std::sync::MutexGuard<'_, Running> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Ends the running session (its connection closes, so the agent shows
    /// the saved configuration again) and returns the next number.
    fn replace(&self) -> u64 {
        let mut running = self.lock();
        if let Some(task) = running.task.take() {
            task.abort();
        }
        running.inbox = None;
        running.number += 1;
        running.number
    }

    /// Ends the running session, if any: the main window reloads or
    /// closes.
    pub fn stop(&self) {
        let mut running = self.lock();
        if let Some(task) = running.task.take() {
            task.abort();
        }
        running.inbox = None;
    }

    fn send(&self, session: u64, change: Change) -> bool {
        let running = self.lock();
        running.number == session
            && running
                .inbox
                .as_ref()
                .is_some_and(|inbox| inbox.send(change).is_ok())
    }
}

/// What the editor hands over.
#[derive(Debug, Clone)]
pub(crate) enum Change {
    Draft {
        revision: u64,
        layout: Box<DeviceLayout>,
    },
    Navigate(EditorNavigation),
}

/// What is sent next and what the agent has: the latest draft only, as
/// parts of what changed since the agent last applied one; drafts before
/// navigations.
#[derive(Debug, Default)]
pub(crate) struct Outbox {
    /// The editor's latest draft.
    latest: Option<(u64, DeviceLayout)>,
    /// The revision last sent completely (or given up).
    sent: u64,
    /// What the agent applied last: the base of the next parts.
    acked: Option<DeviceLayout>,
    /// The revision on its way: its remaining parts and the layout.
    parts: VecDeque<(LayoutPatch, bool)>,
    sending: Option<(u64, DeviceLayout)>,
    /// A navigation to send after the drafts.
    navigation: Option<EditorNavigation>,
    /// Where the editor (or the device) was last: shown again after the
    /// agent came back.
    place: Option<EditorNavigation>,
}

/// A request of the outbox and what its answer settles.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Sent {
    Part { revision: u64, last: bool },
    Navigation,
}

impl Outbox {
    pub(crate) fn push(&mut self, change: Change) {
        match change {
            Change::Draft { revision, layout } => {
                if self.latest.as_ref().is_none_or(|(r, _)| revision > *r) {
                    self.latest = Some((revision, *layout));
                }
            }
            Change::Navigate(navigation) => {
                self.place = Some(navigation.clone());
                self.navigation = Some(navigation);
            }
        }
    }

    /// The device went somewhere by itself: shown there again after a
    /// reconnection.
    pub(crate) fn device_moved(&mut self, navigation: &DeviceNavigation) {
        if let Some(layout) = navigation.layout
            && !navigation.stack.is_empty()
        {
            self.place = Some(EditorNavigation::Show {
                layout,
                stack: navigation.stack.clone(),
            });
        }
    }

    /// The next request for `session`, if anything is to be sent; `Err`
    /// when the latest draft cannot be sent (a page too large).
    pub(crate) fn next(&mut self, session: u64) -> Result<Option<(Request, Sent)>, String> {
        if self.parts.is_empty()
            && let Some((revision, layout)) = self.latest.as_ref()
            && *revision > self.sent
        {
            let (revision, layout) = (*revision, layout.clone());
            self.sent = revision;
            if self.acked.as_ref() != Some(&layout) {
                let parts = patch(self.acked.as_ref(), &layout, PART_BUDGET)
                    .map_err(|error| error.to_string())?;
                let count = parts.len();
                self.parts = parts
                    .into_iter()
                    .enumerate()
                    .map(|(index, part)| (part, index + 1 == count))
                    .collect();
                self.sending = Some((revision, layout));
            }
        }
        if let Some((part, last)) = self.parts.pop_front() {
            let revision = self.sending.as_ref().map_or(0, |(r, _)| *r);
            return Ok(Some((
                Request::ApplyLivePreview {
                    session,
                    revision,
                    part: Box::new(part),
                    last,
                },
                Sent::Part { revision, last },
            )));
        }
        Ok(self.navigation.take().map(|navigation| {
            (
                Request::NavigateLivePreview {
                    session,
                    navigation,
                },
                Sent::Navigation,
            )
        }))
    }

    /// The agent applied `revision`.
    pub(crate) fn applied(&mut self, revision: u64) {
        if let Some((sending, layout)) = self.sending.take() {
            if sending == revision {
                self.acked = Some(layout);
            } else {
                self.sending = Some((sending, layout));
            }
        }
    }

    /// The revision on its way was refused: its remaining parts are
    /// dropped; the next draft is sent against what the agent has.
    pub(crate) fn refused(&mut self) {
        self.parts.clear();
        self.sending = None;
    }

    /// A new connection: everything is sent again, the latest draft in
    /// full, then the place.
    pub(crate) fn reconnected(&mut self) {
        self.parts.clear();
        self.sending = None;
        self.acked = None;
        self.sent = 0;
        self.navigation.clone_from(&self.place);
    }
}

/// Begins a live preview of `layout` of `profile` on `device`, replacing
/// the running one; returns the session's number, which its events and
/// updates carry. Nothing reaches the device until the first draft.
#[tauri::command]
pub fn live_preview_begin<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    live: State<'_, LivePreview>,
    device: DeviceId,
    profile_id: ProfileId,
    layout_id: LayoutId,
) -> u64 {
    let number = live.replace();
    let (inbox, changes) = mpsc::unbounded_channel();
    let paths = state.paths.clone();
    let scope = Scope {
        device,
        profile: profile_id,
        layout: layout_id,
    };
    let task = tauri::async_runtime::spawn(async move {
        let emit = |update: LivePreviewUpdate| {
            let _ = app.emit(
                LIVE_PREVIEW_EVENT,
                LivePreviewEvent {
                    session: number,
                    update,
                },
            );
        };
        run(&paths, &scope, changes, &emit).await;
    });
    let mut running = live.lock();
    if running.number == number {
        running.task = Some(task);
        running.inbox = Some(inbox);
    } else {
        task.abort();
    }
    number
}

/// The editor's latest draft (`revision` grows with every one) and/or a
/// navigation of the editor, for `session`; `false` when that session is
/// no longer the running one.
#[tauri::command]
pub fn live_preview_update(
    live: State<'_, LivePreview>,
    session: u64,
    revision: Option<u64>,
    layout: Option<DeviceLayout>,
    navigation: Option<EditorNavigation>,
) -> bool {
    let mut sent = true;
    if let (Some(revision), Some(layout)) = (revision, layout) {
        sent &= live.send(
            session,
            Change::Draft {
                revision,
                layout: Box::new(layout),
            },
        );
    }
    if let Some(navigation) = navigation {
        sent &= live.send(session, Change::Navigate(navigation));
    }
    sent
}

/// Ends `session` if it is the running one: the device shows the saved
/// configuration again.
#[tauri::command]
pub fn live_preview_end(live: State<'_, LivePreview>, session: u64) {
    if live.lock().number == session {
        live.stop();
    }
}

struct Scope {
    device: DeviceId,
    profile: ProfileId,
    layout: LayoutId,
}

/// How a connection ended.
enum Ended {
    /// The editor ended the session.
    Finished,
    /// The agent went away or ended the session: begin again.
    Again,
}

async fn run(
    paths: &AgentPaths,
    scope: &Scope,
    mut changes: mpsc::UnboundedReceiver<Change>,
    emit: &impl Fn(LivePreviewUpdate),
) {
    let mut outbox = Outbox::default();
    let mut resumed = false;
    loop {
        match connect(paths, scope, &mut changes, &mut outbox, resumed, emit).await {
            Ok(Ended::Finished) => return,
            Ok(Ended::Again) => {}
            Err(_) => emit(LivePreviewUpdate::Disconnected),
        }
        resumed = true;
        outbox.reconnected();
        // Keep taking the editor's changes meanwhile: the latest is sent.
        let pause = tokio::time::sleep(RECONNECT_DELAY);
        tokio::pin!(pause);
        loop {
            tokio::select! {
                () = &mut pause => break,
                change = changes.recv() => match change {
                    Some(change) => outbox.push(change),
                    None => return,
                },
            }
        }
    }
}

/// One connection's session, until it ends.
async fn connect(
    paths: &AgentPaths,
    scope: &Scope,
    changes: &mut mpsc::UnboundedReceiver<Change>,
    outbox: &mut Outbox,
    resumed: bool,
    emit: &impl Fn(LivePreviewUpdate),
) -> Result<Ended, ClientError> {
    let mut client = AgentClient::connect(paths, ClientKind::Desktop).await?;
    if !client.supports(Capability::LivePreview) {
        emit(LivePreviewUpdate::Unsupported);
        // Nothing to do until the editor ends the session.
        while changes.recv().await.is_some() {}
        return Ok(Ended::Finished);
    }
    let (session, navigation) = begin(&mut client, scope).await?;
    outbox.device_moved(&navigation);
    emit(LivePreviewUpdate::Started {
        navigation,
        resumed,
    });
    let (mut writer, mut messages) = client.split();
    let mut waiting: Option<(u64, Sent)> = None;
    loop {
        if waiting.is_none() {
            match outbox.next(session) {
                Ok(Some((request, sent))) => {
                    waiting = Some((writer.send(request).await?, sent));
                }
                Ok(None) => {}
                Err(message) => emit(LivePreviewUpdate::Refused {
                    reason: Some(reason::TOO_LARGE.into()),
                    message,
                }),
            }
        }
        tokio::select! {
            change = changes.recv() => match change {
                Some(change) => outbox.push(change),
                // Ended by the editor: dropping the connection ends it on
                // the agent.
                None => return Ok(Ended::Finished),
            },
            message = messages.recv() => {
                let message = message.unwrap_or(Err(ClientError::Closed))?;
                match message {
                    ServerMessage::Response { id, response } if waiting.as_ref().is_some_and(|(w, _)| *w == id) => {
                        let (_, sent) = waiting.take().expect("checked");
                        answered(response, &sent, outbox, emit);
                    }
                    ServerMessage::Error { id, error } if id.is_none() || waiting.as_ref().map(|(w, _)| *w) == id => {
                        if waiting.take().is_some_and(|(_, sent)| matches!(sent, Sent::Part { .. })) {
                            outbox.refused();
                        }
                        emit(LivePreviewUpdate::Refused {
                            reason: error.reason.clone(),
                            message: error.message.clone(),
                        });
                    }
                    ServerMessage::LivePreview { event: AgentLiveEvent::Navigated { session: of, navigation } } if of == session => {
                        outbox.device_moved(&navigation);
                        emit(LivePreviewUpdate::Navigated { navigation, own: false });
                    }
                    ServerMessage::LivePreview { event: AgentLiveEvent::Ended { session: of, reason } } if of == session => {
                        let reason = serde_json::to_value(reason)
                            .ok()
                            .and_then(|value| value.as_str().map(str::to_owned))
                            .unwrap_or_default();
                        emit(LivePreviewUpdate::Ended { reason });
                        return Ok(Ended::Again);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn answered(
    response: Response,
    sent: &Sent,
    outbox: &mut Outbox,
    emit: &impl Fn(LivePreviewUpdate),
) {
    match (response, sent) {
        (
            Response::LivePreviewApplied {
                revision, applied, ..
            },
            Sent::Part { .. },
        ) => {
            if applied {
                outbox.applied(revision);
                emit(LivePreviewUpdate::Applied { revision });
            }
        }
        (Response::LivePreviewNavigated { navigation, .. }, Sent::Navigation) => {
            outbox.device_moved(&navigation);
            emit(LivePreviewUpdate::Navigated {
                navigation,
                own: true,
            });
        }
        _ => {}
    }
}

/// Begins the session, asking again for a moment while the device is
/// still the previous session's (its connection is closing).
async fn begin(
    client: &mut AgentClient,
    scope: &Scope,
) -> Result<(u64, DeviceNavigation), ClientError> {
    let mut attempts = 0;
    loop {
        let result = client
            .request(Request::BeginLivePreview {
                device: scope.device.clone(),
                profile: scope.profile,
                layout: scope.layout,
                navigation: None,
            })
            .await;
        match result {
            Ok(Response::LivePreviewStarted {
                session,
                navigation,
            }) => return Ok((session, navigation)),
            Ok(other) => return Err(ClientError::Protocol(format!("{other:?}"))),
            Err(ClientError::Agent(error))
                if error.reason.as_deref() == Some(reason::DEVICE_BUSY)
                    && attempts < BUSY_RETRIES =>
            {
                attempts += 1;
                tokio::time::sleep(BUSY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::LayoutPage;

    fn layout() -> DeviceLayout {
        let mut layout = DeviceLayout::new(
            "Main",
            deckpilot_device::DeviceModelId::new("elgato.stream-deck-mk2"),
        );
        layout.pages.push(LayoutPage::new("Two"));
        layout
    }

    fn draft(revision: u64, layout: &DeviceLayout) -> Change {
        Change::Draft {
            revision,
            layout: Box::new(layout.clone()),
        }
    }

    fn part_of(request: &Request) -> (u64, usize, bool) {
        match request {
            Request::ApplyLivePreview {
                revision,
                part,
                last,
                ..
            } => (*revision, part.pages.len(), *last),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn only_the_latest_draft_is_sent_and_then_only_what_changed() {
        let mut outbox = Outbox::default();
        let mut base = layout();
        outbox.push(draft(1, &base));
        base.name = "Renamed".into();
        outbox.push(draft(2, &base));
        // An older draft arriving late is ignored.
        outbox.push(draft(1, &layout()));
        let (request, sent) = outbox.next(7).unwrap().unwrap();
        assert_eq!(
            part_of(&request),
            (2, 2, true),
            "everything, the first time"
        );
        assert_eq!(
            sent,
            Sent::Part {
                revision: 2,
                last: true
            }
        );
        assert!(outbox.next(7).unwrap().is_none());
        outbox.applied(2);
        // One page changed: one page sent.
        base.pages[1].name = "Changed".into();
        outbox.push(draft(3, &base));
        let (request, _) = outbox.next(7).unwrap().unwrap();
        assert_eq!(part_of(&request), (3, 1, true));
        outbox.applied(3);
        // The same layout again: nothing.
        outbox.push(draft(4, &base));
        assert!(outbox.next(7).unwrap().is_none());
    }

    #[test]
    fn a_navigation_goes_after_the_draft_it_needs() {
        let mut outbox = Outbox::default();
        let mut base = layout();
        let fresh = LayoutPage::new("Fresh");
        let fresh_id = fresh.id;
        base.pages.push(fresh);
        outbox.push(draft(1, &base));
        outbox.push(Change::Navigate(EditorNavigation::Show {
            layout: base.id,
            stack: vec![fresh_id],
        }));
        let (first, _) = outbox.next(1).unwrap().unwrap();
        assert!(matches!(first, Request::ApplyLivePreview { .. }));
        let (second, sent) = outbox.next(1).unwrap().unwrap();
        assert!(matches!(second, Request::NavigateLivePreview { .. }));
        assert_eq!(sent, Sent::Navigation);
        assert!(outbox.next(1).unwrap().is_none());
    }

    #[test]
    fn after_a_reconnection_the_latest_draft_and_the_place_are_sent_again() {
        let mut outbox = Outbox::default();
        let base = layout();
        outbox.push(draft(5, &base));
        outbox.next(1).unwrap().unwrap();
        outbox.applied(5);
        outbox.device_moved(&DeviceNavigation {
            profile: None,
            layout: Some(base.id),
            stack: vec![base.home_page],
            connected: true,
            in_scope: true,
            seq: 3,
        });
        assert!(outbox.next(1).unwrap().is_none());
        outbox.reconnected();
        let (request, _) = outbox.next(2).unwrap().unwrap();
        assert_eq!(part_of(&request), (5, 2, true), "in full");
        let (place, _) = outbox.next(2).unwrap().unwrap();
        assert_eq!(
            place,
            Request::NavigateLivePreview {
                session: 2,
                navigation: EditorNavigation::Show {
                    layout: base.id,
                    stack: vec![base.home_page],
                },
            }
        );
    }

    #[test]
    fn a_refused_revision_is_followed_by_the_next_against_what_the_agent_has() {
        let mut outbox = Outbox::default();
        let mut base = layout();
        outbox.push(draft(1, &base));
        outbox.next(1).unwrap().unwrap();
        outbox.applied(1);
        base.pages[0].name = "Bad".into();
        outbox.push(draft(2, &base));
        outbox.next(1).unwrap().unwrap();
        outbox.refused();
        base.pages[0].name = "Good".into();
        outbox.push(draft(3, &base));
        let (request, _) = outbox.next(1).unwrap().unwrap();
        assert_eq!(part_of(&request), (3, 1, true));
    }

    #[test]
    fn a_page_too_large_is_refused_here() {
        let mut outbox = Outbox::default();
        let mut base = layout();
        base.pages[0].name = "x".repeat(PART_BUDGET + 1);
        outbox.push(draft(1, &base));
        assert!(outbox.next(1).is_err());
        // Not tried again until the next draft.
        assert!(outbox.next(1).unwrap().is_none());
    }
}
