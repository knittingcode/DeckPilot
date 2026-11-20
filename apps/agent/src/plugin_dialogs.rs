//! Plugin dialogs between the plugin executors and the app (protocol 12,
//! plugin API 1.5, `docs/dialogs.md`).
//!
//! An executor (on a plugin's lane thread) opens a dialog here and waits
//! for its [`Outcome`]; the app connections that announced
//! [`FEATURE`](deckpilot_ipc::plugin_dialogs::FEATURE) receive it as a
//! `dialogOpened` plugin event and answer with `answerPluginDialog`.
//!
//! * **Only announcing connections.** Dialogs go only to connections that
//!   announced the feature ([`PluginDialogs::app_connected`]); without one
//!   a dialog is refused at once ([`OpenError::Unavailable`]), and when the
//!   last one closes every open dialog ends `appClosed`. Only such a
//!   connection may answer.
//! * **Answers are checked** against the dialog before they reach the
//!   plugin (the checker the executor passed in, built from the plugin's
//!   request); an answer that does not fit is refused and the dialog stays
//!   open.
//! * **Isolation.** Ids are random (issued by the executor from the CSPRNG)
//!   and known only to the agent and the app; an answer names the dialog
//!   and its plugin, and one that does not match an open dialog of that
//!   plugin is refused as not found. Each dialog has its own reply
//!   channel, so an answer only ever reaches the invocation that asked.
//!   Plugins have no operation to list, close or read dialogs.
//! * **One per plugin.** A plugin with an open dialog gets
//!   [`OpenError::AlreadyOpen`].

// Without the `plugins` feature nothing opens a dialog: only the app side
// (announcing, answering) is used.
#![cfg_attr(not(feature = "plugins"), allow(dead_code, unused_imports))]

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};

use deckpilot_ipc::plugin_dialogs::{
    AppCancelReason, DialogAnswer, DialogCloseReason, DialogSpec, PluginDialog,
};
use deckpilot_ipc::plugins::PluginEvent;
use deckpilot_ipc::{ErrorCode, IpcError};
use serde_json::{Map, Value};
use tokio::sync::broadcast;

/// Checks a confirmed answer (`values`, `selected`) against the dialog and
/// builds what the plugin receives, or says why it does not fit.
pub(crate) type AnswerCheck =
    Box<dyn Fn(Option<&Map<String, Value>>, Option<&str>) -> Result<Value, String> + Send>;

/// How a dialog ended, for the invocation that waits for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Confirmed: what the checker built.
    Confirmed(Value),
    /// Cancelled by the app (the user, or the app is closing).
    Cancelled(AppCancelReason),
}

/// Why a dialog could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenError {
    /// No app connection shows plugin dialogs.
    Unavailable,
    /// The plugin already has an open dialog.
    AlreadyOpen,
}

struct Open {
    plugin: String,
    check: AnswerCheck,
    reply: SyncSender<Outcome>,
}

#[derive(Default)]
struct Inner {
    /// App connections that announced the feature.
    apps: usize,
    open: BTreeMap<String, Open>,
}

/// The open plugin dialogs and the app connections that show them.
pub struct PluginDialogs {
    inner: Mutex<Inner>,
    events: broadcast::Sender<PluginEvent>,
}

impl std::fmt::Debug for PluginDialogs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("PluginDialogs")
            .field("apps", &inner.apps)
            .field("open", &inner.open.len())
            .finish()
    }
}

impl Default for PluginDialogs {
    fn default() -> Self {
        Self {
            inner: Mutex::default(),
            events: broadcast::channel(64).0,
        }
    }
}

/// An app connection that shows plugin dialogs; dropping it (the connection
/// closed) ends every open dialog `appClosed` when it was the last one.
pub(crate) struct AppGuard(Arc<PluginDialogs>);

impl Drop for AppGuard {
    fn drop(&mut self) {
        let ended = {
            let mut inner = self.0.lock();
            inner.apps = inner.apps.saturating_sub(1);
            if inner.apps == 0 {
                std::mem::take(&mut inner.open)
            } else {
                BTreeMap::new()
            }
        };
        for (id, open) in ended {
            let _ = open
                .reply
                .try_send(Outcome::Cancelled(AppCancelReason::AppClosed));
            self.0.closed(id, open.plugin, DialogCloseReason::AppClosed);
        }
    }
}

impl PluginDialogs {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn closed(&self, dialog: String, plugin: String, reason: DialogCloseReason) {
        let _ = self.events.send(PluginEvent::DialogClosed {
            dialog,
            plugin,
            reason,
        });
    }

    /// A connection announced the feature: it shows dialogs until the
    /// guard is dropped.
    pub(crate) fn app_connected(self: &Arc<Self>) -> AppGuard {
        self.lock().apps += 1;
        AppGuard(self.clone())
    }

    /// The dialog events, for a connection that announced the feature.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<PluginEvent> {
        self.events.subscribe()
    }

    /// Whether an app shows plugin dialogs now.
    #[cfg(test)]
    pub(crate) fn available(&self) -> bool {
        self.lock().apps > 0
    }

    /// Opens a dialog `id` for `plugin` (shown as `plugin_name`) and sends
    /// it to the apps. The receiver gets its outcome once; it is closed
    /// without one when the dialog ended otherwise.
    pub(crate) fn open(
        &self,
        id: String,
        plugin: &str,
        plugin_name: &str,
        spec: DialogSpec,
        check: AnswerCheck,
    ) -> Result<Receiver<Outcome>, OpenError> {
        let (reply, outcome) = sync_channel(1);
        {
            let mut inner = self.lock();
            if inner.apps == 0 {
                return Err(OpenError::Unavailable);
            }
            if inner.open.values().any(|open| open.plugin == plugin) {
                return Err(OpenError::AlreadyOpen);
            }
            inner.open.insert(
                id.clone(),
                Open {
                    plugin: plugin.to_owned(),
                    check,
                    reply,
                },
            );
        }
        let _ = self.events.send(PluginEvent::DialogOpened {
            dialog: PluginDialog {
                id,
                plugin: plugin.to_owned(),
                plugin_name: plugin_name.to_owned(),
                spec,
            },
        });
        Ok(outcome)
    }

    /// Ends dialog `id` without an answer (its time ran out, the plugin
    /// stopped): the apps take it down. Nothing happens when it already
    /// ended.
    pub(crate) fn close(&self, id: &str, reason: DialogCloseReason) {
        let removed = self.lock().open.remove(id);
        if let Some(open) = removed {
            self.closed(id.to_owned(), open.plugin, reason);
        }
    }

    /// Ends every dialog of `plugin` (it was disabled, updated or
    /// uninstalled): its invocation gets no answer.
    pub(crate) fn close_plugin(&self, plugin: &str) {
        let ended: Vec<(String, Open)> = {
            let mut inner = self.lock();
            let ids: Vec<String> = inner
                .open
                .iter()
                .filter(|(_, open)| open.plugin == plugin)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| inner.open.remove(&id).map(|open| (id, open)))
                .collect()
        };
        for (id, open) in ended {
            self.closed(id, open.plugin, DialogCloseReason::PluginStopped);
        }
    }

    /// The app's answer to dialog `dialog` of `plugin`. `announced`: the
    /// connection announced the feature (others may not answer).
    pub(crate) fn answer(
        &self,
        announced: bool,
        dialog: &str,
        plugin: &str,
        answer: DialogAnswer,
    ) -> Result<(), IpcError> {
        if !announced {
            return Err(IpcError::new(
                ErrorCode::Unsupported,
                "this connection does not show plugin dialogs (announce `pluginDialogs` in \
                 subscribePluginEvents)",
            ));
        }
        let mut inner = self.lock();
        // Another plugin's id, or one that ended: the same answer, so an
        // answer reveals nothing about other dialogs.
        let Some(open) = inner.open.get(dialog).filter(|open| open.plugin == plugin) else {
            return Err(IpcError::new(
                ErrorCode::NotFound,
                "the dialog is no longer open",
            ));
        };
        let outcome = match answer {
            DialogAnswer::Confirmed { values, selected } => {
                match (open.check)(
                    values.as_ref().map(|v| Map::from_iter(v.clone())).as_ref(),
                    selected.as_deref(),
                ) {
                    Ok(value) => Outcome::Confirmed(value),
                    // The dialog stays open for another try.
                    Err(why) => return Err(IpcError::new(ErrorCode::InvalidRequest, why)),
                }
            }
            DialogAnswer::Cancelled { reason } => Outcome::Cancelled(reason),
        };
        let open = inner.open.remove(dialog).expect("found above");
        drop(inner);
        let _ = open.reply.try_send(outcome);
        self.closed(dialog.to_owned(), open.plugin, DialogCloseReason::Answered);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_ipc::plugin_dialogs::DialogType;
    use serde_json::json;

    fn spec() -> DialogSpec {
        DialogSpec {
            kind: DialogType::Confirm,
            title: "Pull now?".into(),
            message: None,
            fields: Vec::new(),
            options: Vec::new(),
            confirm_label: None,
            cancel_label: None,
            destructive: false,
            timeout_seconds: 60,
        }
    }

    /// Accepts `{"ok": true}` values only.
    fn check() -> AnswerCheck {
        Box::new(|values, selected| match (values, selected) {
            (Some(v), None) if v.get("ok") == Some(&json!(true)) => {
                Ok(json!({"outcome": "confirmed", "values": v}))
            }
            _ => Err("the answer does not fit the dialog".into()),
        })
    }

    fn confirmed(values: Value) -> DialogAnswer {
        let Value::Object(values) = values else {
            panic!("values are an object")
        };
        DialogAnswer::Confirmed {
            values: Some(values.into_iter().collect()),
            selected: None,
        }
    }

    fn drain(events: &mut broadcast::Receiver<PluginEvent>) -> Vec<PluginEvent> {
        std::iter::from_fn(|| events.try_recv().ok()).collect()
    }

    #[test]
    fn without_an_app_a_dialog_is_unavailable() {
        let dialogs = Arc::new(PluginDialogs::default());
        assert!(!dialogs.available());
        assert_eq!(
            dialogs
                .open("a1".into(), "acme.repo", "Repo", spec(), check())
                .unwrap_err(),
            OpenError::Unavailable
        );
        let guard = dialogs.app_connected();
        assert!(dialogs.available());
        drop(guard);
        assert!(!dialogs.available());
    }

    #[test]
    fn answers_are_checked_and_reach_only_their_dialog() {
        let dialogs = Arc::new(PluginDialogs::default());
        let _app = dialogs.app_connected();
        let mut events = dialogs.subscribe();
        let first = dialogs
            .open("a1".into(), "acme.repo", "Repo tools", spec(), check())
            .unwrap();
        let second = dialogs
            .open("b2".into(), "acme.other", "Other", spec(), check())
            .unwrap();
        let opened = drain(&mut events);
        assert!(matches!(&opened[0], PluginEvent::DialogOpened { dialog }
            if dialog.id == "a1" && dialog.plugin == "acme.repo" && dialog.plugin_name == "Repo tools"));
        assert_eq!(opened.len(), 2);

        // One open dialog per plugin.
        assert_eq!(
            dialogs
                .open("a3".into(), "acme.repo", "Repo tools", spec(), check())
                .unwrap_err(),
            OpenError::AlreadyOpen
        );

        // A connection that did not announce the feature may not answer.
        assert_eq!(
            dialogs
                .answer(false, "a1", "acme.repo", confirmed(json!({"ok": true})))
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        // Another plugin's dialog, or an unknown id: not found, and nothing
        // reaches either invocation.
        for (dialog, plugin) in [
            ("b2", "acme.repo"),
            ("a1", "acme.other"),
            ("zz", "acme.repo"),
        ] {
            let error = dialogs
                .answer(true, dialog, plugin, confirmed(json!({"ok": true})))
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::NotFound, "{dialog} {plugin}");
        }
        assert!(first.try_recv().is_err());
        assert!(second.try_recv().is_err());

        // An answer that does not fit: refused, the dialog stays open.
        let error = dialogs
            .answer(true, "a1", "acme.repo", confirmed(json!({"ok": false})))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(first.try_recv().is_err());

        dialogs
            .answer(true, "a1", "acme.repo", confirmed(json!({"ok": true})))
            .unwrap();
        assert_eq!(
            first.try_recv().unwrap(),
            Outcome::Confirmed(json!({"outcome": "confirmed", "values": {"ok": true}}))
        );
        assert!(second.try_recv().is_err());
        assert!(
            matches!(&drain(&mut events)[..], [PluginEvent::DialogClosed { dialog, reason, .. }]
            if dialog == "a1" && *reason == DialogCloseReason::Answered)
        );
        // Answered once: a late second answer is not found.
        assert_eq!(
            dialogs
                .answer(true, "a1", "acme.repo", confirmed(json!({"ok": true})))
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        // Now the plugin may open another one.
        assert!(
            dialogs
                .open("a4".into(), "acme.repo", "Repo tools", spec(), check())
                .is_ok()
        );

        dialogs
            .answer(
                true,
                "b2",
                "acme.other",
                DialogAnswer::Cancelled {
                    reason: AppCancelReason::User,
                },
            )
            .unwrap();
        assert_eq!(
            second.try_recv().unwrap(),
            Outcome::Cancelled(AppCancelReason::User)
        );
    }

    #[test]
    fn stopping_a_plugin_and_closing_the_app_end_its_dialogs() {
        let dialogs = Arc::new(PluginDialogs::default());
        let app = dialogs.app_connected();
        let second_app = dialogs.app_connected();
        let mut events = dialogs.subscribe();
        let repo = dialogs
            .open("a1".into(), "acme.repo", "Repo", spec(), check())
            .unwrap();
        let other = dialogs
            .open("b2".into(), "acme.other", "Other", spec(), check())
            .unwrap();
        drain(&mut events);

        dialogs.close_plugin("acme.repo");
        assert!(
            matches!(&drain(&mut events)[..], [PluginEvent::DialogClosed { dialog, plugin, reason }]
            if dialog == "a1" && plugin == "acme.repo" && *reason == DialogCloseReason::PluginStopped)
        );
        // Its invocation gets no answer: the channel is closed.
        assert!(matches!(
            repo.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
        dialogs.close("a1", DialogCloseReason::Timeout);
        assert_eq!(drain(&mut events), Vec::new(), "closed once");

        // One app of two leaves: the dialog stays.
        drop(second_app);
        assert!(other.try_recv().is_err());
        assert_eq!(drain(&mut events), Vec::new());
        // The last one leaves: `appClosed`.
        drop(app);
        assert_eq!(
            other.try_recv().unwrap(),
            Outcome::Cancelled(AppCancelReason::AppClosed)
        );
        assert!(
            matches!(&drain(&mut events)[..], [PluginEvent::DialogClosed { reason, .. }]
            if *reason == DialogCloseReason::AppClosed)
        );
    }
}
