//! Plugin answers that fit one IPC message ([`MAX_MESSAGE_BYTES`]).
//!
//! A plugin's answer grows with its actions (every action's definition:
//! parameters, kinds, outputs, node contract, and its manifest entry) and
//! its icon (up to [`plugins::MAX_ICON_BYTES`], base64 in the answer), so
//! one answer cannot carry them all. An agent that lists
//! [`plugins::feature::PAGED_ANSWERS`] in its runtime status splits them:
//!
//! * `listPluginsPaged` and `getPluginPaged` answer as `listPlugins` and
//!   `getPlugin` do, but without the actions (in the summary and in the
//!   manifest) and the icon; a paged summary has
//!   [`PluginSummary::action_count`] instead;
//! * `getPluginActions` pages the actions, as many per answer as fit
//!   ([`page_actions`]), each with its definition and manifest entry;
//! * `getPluginIcon` sends the icon in chunks of
//!   [`plugins::ICON_CHUNK_BYTES`].
//!
//! [`list_plugins`], [`plugin_details`] and [`plugin_icon`] read them and
//! put the full answers back together, exactly as a single answer had
//! them, so that whoever uses them sees no difference. They retry when the
//! plugin changes while it is read (another version between two pages, a
//! new icon between two chunks).
//!
//! A client that does not use them (an older app) still gets an answer:
//! [`fit`] shortens one that does not fit (the icon, then the definitions
//! of the largest plugins, then the manifest's actions are left out) and
//! refuses one that still does not with [`ErrorCode::Failed`] and the
//! reason [`plugins::reason::ANSWER_TOO_LARGE`], which every client of
//! protocol 10 can show. The connection stays open either way; before,
//! the agent could not write the answer and closed it.

use std::io;

use serde::Serialize;
use serde_json::Value;

use crate::framing::MAX_MESSAGE_BYTES;
use crate::plugins::{
    self, ICON_CHUNK_BYTES, MAX_ICON_BYTES, PluginActionEntry, PluginDetails, PluginSummary,
};
use crate::{AgentClient, ClientError, ErrorCode, IpcError, Request, Response, ServerMessage};

/// What the items of one page may take of a message; the rest is for the
/// envelope (type, id, plugin id, version, counts).
pub const PAGE_BUDGET: usize = MAX_MESSAGE_BYTES - 2 * 1024;

/// How often a reader starts over when the plugin changed while it read.
const ATTEMPTS: usize = 3;

/// The size of `value` as JSON, without writing it anywhere.
pub fn json_len<T: Serialize + ?Sized>(value: &T) -> usize {
    struct Count(usize);
    impl io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    match serde_json::to_writer(&mut count, value) {
        Ok(()) => count.0,
        Err(_) => usize::MAX,
    }
}

// ----------------------------------------------------------- agent side

/// `summary` as a paged answer has it: no actions, their number instead.
pub fn paged_summary(summary: &mut PluginSummary) {
    summary.action_count = Some(u32::try_from(summary.actions.len()).unwrap_or(u32::MAX));
    summary.actions.clear();
}

/// `details` as a paged answer has it: a paged summary, no icon, and no
/// actions in the manifest.
pub fn paged_details(details: &mut PluginDetails) {
    paged_summary(&mut details.summary);
    details.icon = None;
    remove_manifest_actions(&mut details.manifest);
}

/// Removes `contributes.actions`; whether there was one.
fn remove_manifest_actions(manifest: &mut Value) -> bool {
    manifest
        .get_mut("contributes")
        .and_then(Value::as_object_mut)
        .and_then(|contributes| contributes.remove("actions"))
        .is_some()
}

/// The entries from index `from` on that fit [`PAGE_BUDGET`], and the
/// index of the first one left for the next page. A page always has at
/// least one entry (unless `from` is past the end): one that does not fit
/// a message alone goes without its definition and manifest entry, marked
/// [`PluginActionEntry::too_large`].
pub fn page_actions(
    entries: Vec<PluginActionEntry>,
    from: u32,
) -> (Vec<PluginActionEntry>, Option<u32>) {
    let mut page = Vec::new();
    // `[`, `]`, and a comma per entry.
    let mut used = 2;
    for (index, mut entry) in entries.into_iter().enumerate().skip(from as usize) {
        let mut size = json_len(&entry) + 1;
        if page.is_empty() && used + size > PAGE_BUDGET {
            entry.action.definition = None;
            entry.manifest = None;
            entry.too_large = true;
            size = json_len(&entry) + 1;
        }
        if !page.is_empty() && used + size > PAGE_BUDGET {
            return (page, Some(u32::try_from(index).unwrap_or(u32::MAX)));
        }
        used += size;
        page.push(entry);
    }
    (page, None)
}

/// An answer's message as it is written: borrowing the response, the
/// same JSON as [`ServerMessage::Response`].
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Reply<'a> {
    Response { id: u64, response: &'a Response },
}

/// `message` as it can be sent: unchanged when it fits one message; an
/// answer that does not, shortened until it does ([`shorten`]), or else
/// refused with [`plugins::reason::ANSWER_TOO_LARGE`] for the same
/// request. `None` for anything else that does not fit (an event): it
/// cannot be sent, and the connection goes on without it.
pub fn fit(message: ServerMessage) -> Option<ServerMessage> {
    if json_len(&message) <= MAX_MESSAGE_BYTES {
        return Some(message);
    }
    let ServerMessage::Response { id, mut response } = message else {
        return None;
    };
    let fits =
        |response: &Response| json_len(&Reply::Response { id, response }) <= MAX_MESSAGE_BYTES;
    loop {
        if fits(&response) {
            return Some(ServerMessage::Response { id, response });
        }
        if !shorten(&mut response) {
            return Some(ServerMessage::Error {
                id: Some(id),
                error: too_large(),
            });
        }
    }
}

/// The refusal of an answer that does not fit one message.
pub fn too_large() -> IpcError {
    IpcError::new(
        ErrorCode::Failed,
        format!(
            "the answer does not fit the {} KiB of one IPC message; an app that reads plugin \
             answers in pages can show it",
            MAX_MESSAGE_BYTES / 1024
        ),
    )
    .with_reason(plugins::reason::ANSWER_TOO_LARGE)
}

/// One step of making a plugin answer smaller, in the order the app misses
/// least: a plugin's icon (a placeholder is shown); the definitions of the
/// largest plugin's actions (the editors do not offer them, as for an
/// action whose definition does not convert); the actions in a plugin's
/// manifest (its test form has no parameters). `false` when nothing is
/// left to leave out.
fn shorten(response: &mut Response) -> bool {
    match response {
        Response::Plugin { plugin } => {
            if plugin.icon.take().is_some() {
                return true;
            }
            if without_definitions(&mut plugin.summary) {
                return true;
            }
            remove_manifest_actions(&mut plugin.manifest)
        }
        Response::Plugins { plugins } => plugins
            .iter_mut()
            .filter(|p| p.actions.iter().any(|a| a.definition.is_some()))
            .max_by_key(|p| json_len(&**p))
            .is_some_and(without_definitions),
        _ => false,
    }
}

/// Leaves out every action definition; whether there was one.
fn without_definitions(summary: &mut PluginSummary) -> bool {
    // Every one, not only up to the first (no short-circuit).
    let mut removed = false;
    for action in &mut summary.actions {
        removed |= action.definition.take().is_some();
    }
    removed
}

// ---------------------------------------------------------- client side

fn unexpected(request: &str) -> ClientError {
    ClientError::Protocol(format!("unexpected answer to `{request}`"))
}

fn changing(plugin: &str) -> ClientError {
    ClientError::Protocol(format!("{plugin} kept changing while it was read"))
}

/// The installed plugins in full, as `listPlugins` has them, read with
/// paged answers. Needs an agent with [`plugins::feature::PAGED_ANSWERS`].
pub async fn list_plugins(client: &mut AgentClient) -> Result<Vec<PluginSummary>, ClientError> {
    'attempt: for _ in 0..ATTEMPTS {
        let Response::Plugins { mut plugins } = client.request(Request::ListPluginsPaged).await?
        else {
            return Err(unexpected("listPluginsPaged"));
        };
        for summary in &mut plugins {
            if !complete(client, summary, None).await? {
                continue 'attempt;
            }
        }
        return Ok(plugins);
    }
    Err(changing("a plugin"))
}

/// One installed plugin in full, as `getPlugin` has it (icon included),
/// read with paged answers. Needs an agent with
/// [`plugins::feature::PAGED_ANSWERS`].
pub async fn plugin_details(
    client: &mut AgentClient,
    plugin: &str,
) -> Result<PluginDetails, ClientError> {
    for _ in 0..ATTEMPTS {
        let request = Request::GetPluginPaged {
            plugin: plugin.to_owned(),
        };
        let Response::Plugin { plugin: details } = client.request(request).await? else {
            return Err(unexpected("getPluginPaged"));
        };
        let mut details = *details;
        if !complete(client, &mut details.summary, Some(&mut details.manifest)).await? {
            continue;
        }
        details.icon = plugin_icon(client, plugin).await?;
        return Ok(details);
    }
    Err(changing(plugin))
}

/// Puts a paged summary's actions back (and into `manifest`); `false` when
/// the plugin is now another version than the summary's (read it again).
/// A summary that is not paged stays as it is.
async fn complete(
    client: &mut AgentClient,
    summary: &mut PluginSummary,
    manifest: Option<&mut Value>,
) -> Result<bool, ClientError> {
    match summary.action_count {
        None => return Ok(true),
        Some(0) => {
            summary.action_count = None;
            return Ok(true);
        }
        Some(_) => {}
    }
    let (version, entries) = plugin_actions(client, &summary.id).await?;
    if version != summary.version {
        return Ok(false);
    }
    let mut declared = Vec::new();
    summary.actions = entries
        .into_iter()
        .map(|entry| {
            declared.extend(entry.manifest);
            entry.action
        })
        .collect();
    summary.action_count = None;
    if let Some(contributes) = manifest
        .and_then(|m| m.get_mut("contributes"))
        .and_then(Value::as_object_mut)
        && !declared.is_empty()
    {
        contributes.insert("actions".to_owned(), Value::Array(declared));
    }
    Ok(true)
}

/// Every action of `plugin`, page by page: the version they belong to and
/// the entries in the manifest's order. Starts over when the version
/// changes between pages.
pub async fn plugin_actions(
    client: &mut AgentClient,
    plugin: &str,
) -> Result<(String, Vec<PluginActionEntry>), ClientError> {
    'attempt: for _ in 0..ATTEMPTS {
        let mut from = 0;
        let mut first: Option<(String, u32)> = None;
        let mut entries = Vec::new();
        loop {
            let request = Request::GetPluginActions {
                plugin: plugin.to_owned(),
                from,
            };
            let Response::PluginActions {
                version,
                total,
                actions,
                next,
                ..
            } = client.request(request).await?
            else {
                return Err(unexpected("getPluginActions"));
            };
            match &first {
                None => first = Some((version.clone(), total)),
                Some(seen) if *seen != (version.clone(), total) => continue 'attempt,
                Some(_) => {}
            }
            entries.extend(actions);
            match next {
                None if entries.len() == total as usize => return Ok((version, entries)),
                Some(next) if next > from && next <= total => from = next,
                _ => {
                    return Err(ClientError::Protocol(format!(
                        "the pages of {plugin}'s actions do not add up"
                    )));
                }
            }
        }
    }
    Err(changing(plugin))
}

/// `plugin`'s icon as a `data:image/png;base64,` URL, as
/// [`PluginDetails::icon`] has it, read in chunks; `None` when the agent
/// has none to show. Starts over when the file changes between chunks.
pub async fn plugin_icon(
    client: &mut AgentClient,
    plugin: &str,
) -> Result<Option<String>, ClientError> {
    read_icon(client, plugin, None, None).await
}

/// The icon of `plugin`'s navigation entry `entry` for `theme` as a
/// `data:` URL (`image/png`, `image/webp` or `image/svg+xml`, an SVG
/// sanitized by the agent), read in chunks; `None` when the agent has none
/// to show (no such entry, a missing, oversized, unsafe or modified file).
/// Only for an agent that lists [`plugins::feature::NAVIGATION`]: an
/// older one would answer with the plugin's own icon.
pub async fn navigation_icon(
    client: &mut AgentClient,
    plugin: &str,
    entry: &str,
    theme: plugins::NavigationIconTheme,
) -> Result<Option<String>, ClientError> {
    let navigation = plugins::NavigationIconRef {
        entry: entry.to_owned(),
        theme,
    };
    read_icon(client, plugin, Some(navigation), None).await
}

/// The file of `plugin`'s visual asset `asset` (its id in the plugin) as a
/// `data:` URL (`image/png` or `image/jpeg`), read in chunks, checked by
/// the agent against the package's integrity record; `None` when the
/// agent has none to show (no such asset, a gradient, a missing, oversized
/// or modified file). Only for an agent that lists
/// [`plugins::feature::VISUAL_ASSETS`]: an older one would answer with the
/// plugin's own icon.
pub async fn visual_asset_file(
    client: &mut AgentClient,
    plugin: &str,
    asset: &str,
) -> Result<Option<String>, ClientError> {
    read_icon(client, plugin, None, Some(asset.to_owned())).await
}

async fn read_icon(
    client: &mut AgentClient,
    plugin: &str,
    navigation: Option<plugins::NavigationIconRef>,
    asset: Option<String>,
) -> Result<Option<String>, ClientError> {
    let broken = || ClientError::Protocol(format!("the chunks of {plugin}'s icon do not add up"));
    let media_ok = |media: &str| match (&navigation, &asset) {
        (_, Some(_)) => plugins::VISUAL_ASSET_MEDIA_TYPES.contains(&media),
        (None, None) => media == "image/png",
        (Some(_), None) => plugins::NAVIGATION_ICON_MEDIA_TYPES.contains(&media),
    };
    let max_bytes = if asset.is_some() {
        plugins::MAX_VISUAL_ASSET_FILE_BYTES
    } else {
        MAX_ICON_BYTES
    };
    'attempt: for _ in 0..ATTEMPTS {
        let mut offset = 0;
        let mut file: Option<(u64, String, String)> = None;
        let mut data = String::new();
        loop {
            let request = Request::GetPluginIcon {
                plugin: plugin.to_owned(),
                offset,
                navigation: navigation.clone(),
                asset: asset.clone(),
            };
            let Response::PluginIcon { icon, .. } = client.request(request).await? else {
                return Err(unexpected("getPluginIcon"));
            };
            let Some(chunk) = icon else {
                return Ok(None);
            };
            if !media_ok(&chunk.media_type)
                || chunk.offset != offset
                || chunk.size > max_bytes as u64
            {
                return Err(broken());
            }
            match &file {
                None => {
                    file = Some((chunk.size, chunk.sha256.clone(), chunk.media_type.clone()));
                }
                Some((size, sha256, media))
                    if *size != chunk.size
                        || *sha256 != chunk.sha256
                        || *media != chunk.media_type =>
                {
                    continue 'attempt;
                }
                Some(_) => {}
            }
            data.push_str(&chunk.data);
            if data.len() > (max_bytes / 3 + 1) * 4 {
                return Err(broken());
            }
            match chunk.next {
                None => {
                    return Ok(Some(format!("data:{};base64,{data}", chunk.media_type)));
                }
                Some(next) if next == offset + ICON_CHUNK_BYTES as u64 && next < chunk.size => {
                    offset = next;
                }
                Some(_) => return Err(broken()),
            }
        }
    }
    Err(changing(plugin))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{PluginActionInfo, PluginOrigin, PluginState};

    fn action(id: &str, outputs: usize) -> PluginActionEntry {
        let definition: deckpilot_core::ActionType = serde_json::from_value(serde_json::json!({
            "id": format!("acme.big.{id}"),
            "name": id,
            "description": "An action with many outputs.",
            "category": "Test",
            "parameters": [],
            "requiresProject": false,
            "node": {
                "cancellable": true,
                "outputs": (0..outputs)
                    .map(|i| serde_json::json!({"key": format!("output{i}"),
                        "label": format!("Output number {i}"), "kind": "text"}))
                    .collect::<Vec<_>>(),
            }
        }))
        .unwrap();
        PluginActionEntry {
            action: PluginActionInfo {
                action: id.into(),
                action_type: format!("acme.big.{id}"),
                name: id.into(),
                description: Some("An action with many outputs.".into()),
                available: true,
                unavailable_reason: None,
                definition: Some(definition),
            },
            manifest: Some(serde_json::json!({"id": id, "pad": "x".repeat(outputs * 60)})),
            too_large: false,
        }
    }

    fn summary(actions: Vec<PluginActionInfo>) -> PluginSummary {
        PluginSummary {
            id: "acme.big".into(),
            name: "Big".into(),
            publisher: "Acme".into(),
            version: "1.0.0".into(),
            state: PluginState::Enabled,
            state_reason: None,
            origin: PluginOrigin::Sideload,
            previous_version: None,
            staged_version: None,
            actions,
            action_count: None,
            health: None,
            navigation: Vec::new(),
            installed_for: None,
            visual_assets: Vec::new(),
        }
    }

    fn details(actions: usize, icon: usize) -> PluginDetails {
        let entries: Vec<_> = (0..actions).map(|i| action(&format!("a{i}"), 20)).collect();
        PluginDetails {
            summary: summary(entries.iter().map(|e| e.action.clone()).collect()),
            description: String::new(),
            license: "MIT".into(),
            homepage: None,
            permissions: vec![],
            pending_update: None,
            manifest: serde_json::json!({
                "id": "acme.big",
                "contributes": {
                    "actions": entries.iter().map(|e| e.manifest.clone()).collect::<Vec<_>>(),
                    "settings": {"fields": []}
                }
            }),
            audit: vec![],
            icon: (icon > 0).then(|| format!("data:image/png;base64,{}", "A".repeat(icon))),
            storage: None,
            dependencies: Vec::new(),
            dependents: Vec::new(),
        }
    }

    #[test]
    fn pages_fill_a_message_and_cover_every_action_once() {
        let entries: Vec<_> = (0..40).map(|i| action(&format!("a{i}"), 20)).collect();
        let mut from = 0;
        let mut seen = Vec::new();
        let mut pages = 0;
        loop {
            let (page, next) = page_actions(entries.clone(), from);
            assert_ne!(page, vec![]);
            let message = ServerMessage::Response {
                id: u64::MAX,
                response: Response::PluginActions {
                    plugin: "a".repeat(plugins::MAX_PLUGIN_TEXT),
                    version: "1000000.1000000.1000000-prerelease".into(),
                    total: 40,
                    actions: page.clone(),
                    next,
                },
            };
            assert!(json_len(&message) <= MAX_MESSAGE_BYTES);
            assert!(page.iter().all(|e| !e.too_large));
            seen.extend(page.into_iter().map(|e| e.action.action));
            pages += 1;
            match next {
                Some(next) => from = next,
                None => break,
            }
        }
        assert!(pages > 1, "40 actions of 20 outputs fit one page?");
        let expected: Vec<String> = (0..40).map(|i| format!("a{i}")).collect();
        assert_eq!(seen, expected);
        assert_eq!(page_actions(entries, 40), (vec![], None));
    }

    #[test]
    fn an_action_too_large_alone_goes_without_its_definition() {
        let mut huge = action("huge", 20);
        huge.manifest = Some(serde_json::json!({"pad": "x".repeat(MAX_MESSAGE_BYTES)}));
        let entries = vec![action("a", 2), huge, action("b", 2)];
        let (page, next) = page_actions(entries.clone(), 0);
        assert_eq!(page.len(), 1);
        assert_eq!(next, Some(1));
        let (page, next) = page_actions(entries.clone(), 1);
        assert_eq!(page.len(), 2, "the next one still fits");
        assert!(page[0].too_large);
        assert_eq!(page[0].action.definition, None);
        assert_eq!(page[0].manifest, None);
        assert_eq!(page[0].action.action, "huge");
        assert!(!page[1].too_large);
        assert_eq!(next, None);
    }

    #[test]
    fn paged_answers_leave_the_actions_and_the_icon_out() {
        let mut paged = details(40, 300_000);
        paged_details(&mut paged);
        assert_eq!(paged.summary.actions, vec![]);
        assert_eq!(paged.summary.action_count, Some(40));
        assert_eq!(paged.icon, None);
        assert!(paged.manifest["contributes"].get("actions").is_none());
        assert!(paged.manifest["contributes"].get("settings").is_some());
        let json = serde_json::to_value(&paged).unwrap();
        assert_eq!(json["summary"]["actionCount"], 40);
        // A summary that is not paged has no count (older clients never see
        // the field).
        let full = details(1, 0).summary;
        assert!(
            serde_json::to_value(&full)
                .unwrap()
                .get("actionCount")
                .is_none()
        );
    }

    fn answer(response: Response) -> ServerMessage {
        ServerMessage::Response { id: 7, response }
    }

    #[test]
    fn an_answer_that_fits_is_sent_as_it_is() {
        let message = answer(Response::Plugin {
            plugin: Box::new(details(3, 1000)),
        });
        assert_eq!(fit(message.clone()), Some(message));
    }

    #[test]
    fn a_large_details_answer_loses_its_icon_first_then_the_definitions() {
        // The icon alone is too much.
        let fitted = fit(answer(Response::Plugin {
            plugin: Box::new(details(3, 200 * 1024)),
        }))
        .unwrap();
        assert!(json_len(&fitted) <= MAX_MESSAGE_BYTES);
        let ServerMessage::Response {
            id: 7,
            response: Response::Plugin { plugin },
        } = fitted
        else {
            panic!("{fitted:?}");
        };
        assert_eq!(plugin.icon, None);
        assert!(
            plugin
                .summary
                .actions
                .iter()
                .all(|a| a.definition.is_some())
        );
        // 40 actions of 20 outputs: no icon, no definitions, and at last
        // no manifest actions.
        let fitted = fit(answer(Response::Plugin {
            plugin: Box::new(details(40, 200 * 1024)),
        }))
        .unwrap();
        assert!(json_len(&fitted) <= MAX_MESSAGE_BYTES);
        let ServerMessage::Response {
            response: Response::Plugin { plugin },
            ..
        } = fitted
        else {
            panic!("{fitted:?}");
        };
        assert_eq!(plugin.icon, None);
        assert_eq!(plugin.summary.actions.len(), 40);
        assert!(
            plugin
                .summary
                .actions
                .iter()
                .all(|a| a.definition.is_none())
        );
    }

    #[test]
    fn a_large_list_loses_the_definitions_of_its_largest_plugins() {
        let big = details(40, 0).summary;
        let small = PluginSummary {
            id: "acme.small".into(),
            ..details(2, 0).summary
        };
        let fitted = fit(answer(Response::Plugins {
            plugins: vec![big, small],
        }))
        .unwrap();
        assert!(json_len(&fitted) <= MAX_MESSAGE_BYTES);
        let ServerMessage::Response {
            response: Response::Plugins { plugins },
            ..
        } = fitted
        else {
            panic!("{fitted:?}");
        };
        assert!(plugins[0].actions.iter().all(|a| a.definition.is_none()));
        assert!(plugins[1].actions.iter().all(|a| a.definition.is_some()));
    }

    #[test]
    fn what_cannot_be_shortened_is_refused_with_a_reason_every_client_reads() {
        let entries: Vec<_> = (0..200).map(|i| action(&format!("a{i}"), 20)).collect();
        let fitted = fit(answer(Response::PluginActions {
            plugin: "acme.big".into(),
            version: "1.0.0".into(),
            total: 200,
            actions: entries,
            next: None,
        }))
        .unwrap();
        let ServerMessage::Error { id: Some(7), error } = &fitted else {
            panic!("{fitted:?}");
        };
        assert_eq!(error.code, ErrorCode::Failed);
        assert_eq!(
            error.reason.as_deref(),
            Some(plugins::reason::ANSWER_TOO_LARGE)
        );
        // An older client reads it: the error shape is the one it knows.
        let json = serde_json::to_value(&fitted).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["error"]["code"], "failed");
        // An event that does not fit is dropped, not the connection.
        let event = ServerMessage::PluginEvent {
            event: plugins::PluginEvent::Progress {
                plugin: "acme.big".into(),
                operation: plugins::PluginOperation::Install,
                stage: plugins::PluginStage::Failed,
                message: Some("x".repeat(MAX_MESSAGE_BYTES)),
            },
        };
        assert_eq!(fit(event), None);
    }

    #[test]
    fn the_original_requests_and_status_are_unchanged() {
        use serde_json::json;
        // The paged requests are new ones: what an older app sends means
        // what it meant.
        for (wire, request) in [
            (json!({"type": "listPlugins"}), Request::ListPlugins),
            (
                json!({"type": "getPlugin", "plugin": "acme.x"}),
                Request::GetPlugin {
                    plugin: "acme.x".into(),
                },
            ),
            (
                json!({"type": "listPluginsPaged"}),
                Request::ListPluginsPaged,
            ),
            (
                json!({"type": "getPluginPaged", "plugin": "acme.x"}),
                Request::GetPluginPaged {
                    plugin: "acme.x".into(),
                },
            ),
        ] {
            assert_eq!(
                serde_json::from_value::<Request>(wire.clone()).unwrap(),
                request
            );
            assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        }
        for request in [
            Request::ListPluginsPaged,
            Request::GetPluginPaged {
                plugin: "acme.x".into(),
            },
            Request::GetPluginActions {
                plugin: "acme.x".into(),
                from: 0,
            },
            Request::GetPluginIcon {
                plugin: "acme.x".into(),
                offset: 0,
                navigation: None,
                asset: None,
            },
        ] {
            assert_eq!(request.since_protocol(), 10);
            assert!(request.validate().is_ok());
        }
        assert_eq!(
            serde_json::from_value::<Request>(json!({"type": "getPluginActions", "plugin": "a.b"}))
                .unwrap(),
            Request::GetPluginActions {
                plugin: "a.b".into(),
                from: 0
            }
        );
        // An agent without the feature reports none.
        let status: plugins::PluginRuntimeStatus = serde_json::from_value(json!({
            "available": true, "built": true, "enabled": true,
            "executionSupported": true, "platform": "macos"
        }))
        .unwrap();
        assert!(!status.has_feature(plugins::feature::PAGED_ANSWERS));
        assert!(
            serde_json::to_value(&status)
                .unwrap()
                .get("features")
                .is_none()
        );
        let newer = plugins::PluginRuntimeStatus {
            features: vec![
                plugins::feature::PAGED_ANSWERS.into(),
                "somethingLater".into(),
            ],
            ..status
        };
        assert!(newer.has_feature(plugins::feature::PAGED_ANSWERS));
    }

    #[test]
    fn the_borrowed_reply_is_the_same_json_as_the_message() {
        let response = Response::Plugin {
            plugin: Box::new(details(2, 10)),
        };
        assert_eq!(
            serde_json::to_vec(&Reply::Response {
                id: 9,
                response: &response
            })
            .unwrap(),
            serde_json::to_vec(&ServerMessage::Response { id: 9, response }).unwrap()
        );
    }

    #[test]
    fn icon_chunks_join_into_the_files_base64() {
        // The chunk size is a multiple of 3: no padding inside the joined
        // text (3 bytes are 4 characters), and the largest chunk's answer
        // fits one message.
        assert_eq!(ICON_CHUNK_BYTES % 3, 0);
        let chunk = "A".repeat(ICON_CHUNK_BYTES / 3 * 4);
        let message = ServerMessage::Response {
            id: u64::MAX,
            response: Response::PluginIcon {
                plugin: "a".repeat(plugins::MAX_PLUGIN_TEXT),
                icon: Some(plugins::PluginIconChunk {
                    media_type: "image/png".into(),
                    size: MAX_ICON_BYTES as u64,
                    sha256: "f".repeat(64),
                    offset: (MAX_ICON_BYTES - ICON_CHUNK_BYTES) as u64,
                    data: chunk,
                    next: Some(MAX_ICON_BYTES as u64),
                }),
            },
        };
        assert!(json_len(&message) <= MAX_MESSAGE_BYTES);
    }
}
