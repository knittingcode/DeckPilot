//! Live preview of the Device Editor's unsaved layout (protocol 16,
//! capability `livePreview`, `docs/editor-navigation-and-simulation.md`).
//!
//! The editor's working layout (its draft) is shown **and run** by one
//! device while a preview session is open, without being saved: the agent
//! keeps it in memory for that device only, and the database is never the
//! preview channel. The session belongs to the connection that began it;
//! when that connection ends (the editor closed, the app quit or crashed),
//! the device shows the saved configuration again.
//!
//! - `beginLivePreview` opens a session for one device, profile and layout;
//!   the working layout starts as the saved one.
//! - `applyLivePreview` sends a revision of the working layout as one or
//!   more [`LayoutPatch`]es (a whole layout may not fit one message):
//!   page-level upserts plus the page order. Parts of one revision are
//!   applied together with the last one; revisions only go forward
//!   ([`RevisionGate`]), so a late, older revision never replaces a newer
//!   one.
//! - `navigateLivePreview` shows a page (or another layout) on the device,
//!   as the editor navigated: never by running a binding.
//! - The agent pushes [`LivePreviewEvent::Navigated`] when the device's
//!   navigation changes by itself (a key, Auto Exit, a reload), so the
//!   editor follows the device.
//! - `endLivePreview` closes the session.

use serde::{Deserialize, Serialize};

use deckpilot_core::{DeviceLayout, LayoutId, LayoutPage, PageId, ProfileId};
use deckpilot_device::ControlId;

/// Most pages a working layout may have.
pub const MAX_PAGES: usize = 512;
/// Deepest page stack a navigation may ask for.
pub const MAX_STACK: usize = 32;
/// Serialized pages per [`LayoutPatch`], leaving room for the request's
/// envelope within one message ([`crate::framing::MAX_MESSAGE_BYTES`]).
pub const PART_BUDGET: usize = 48 * 1024;
/// Most bytes of pages one revision may stage.
pub const MAX_STAGED_BYTES: usize = 4 * 1024 * 1024;

/// Machine-readable reasons of refused live preview requests
/// ([`crate::IpcError::reason`]).
pub mod reason {
    /// A revision not newer than the one applied.
    pub const STALE_REVISION: &str = "livePreview.staleRevision";
    /// No such session on this connection (ended, or of another one).
    pub const UNKNOWN_SESSION: &str = "livePreview.unknownSession";
    /// Another connection previews on this device.
    pub const DEVICE_BUSY: &str = "livePreview.deviceBusy";
    /// The working layout is not a layout of the session's model and id,
    /// or its pages do not add up.
    pub const INVALID_LAYOUT: &str = "livePreview.invalidLayout";
    /// A navigation names a page the layout does not have.
    pub const UNKNOWN_PAGE: &str = "livePreview.unknownPage";
    /// The layout is not one of the profile's for the device's model.
    pub const WRONG_MODEL: &str = "livePreview.wrongModel";
    /// The profile is not the active one.
    pub const INACTIVE_PROFILE: &str = "livePreview.inactiveProfile";
    /// A page or a revision is larger than the limits.
    pub const TOO_LARGE: &str = "livePreview.tooLarge";
}

/// Where the editor shows the device (`navigateLivePreview`, and
/// `beginLivePreview` to put the device where the editor is).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EditorNavigation {
    /// Shows `stack` (folders first, the page shown last) of `layout`; a
    /// layout the device does not show is switched to as *Switch layout*
    /// would (the layout left can be gone back to).
    Show {
        layout: LayoutId,
        stack: Vec<PageId>,
    },
    /// *Back to previous layout*.
    LayoutBack,
}

/// Checks the bounds of a navigation: at least one page, at most
/// [`MAX_STACK`].
pub fn check_navigation(navigation: &EditorNavigation) -> Result<(), String> {
    match navigation {
        EditorNavigation::Show { stack, .. } if stack.is_empty() => {
            Err("a navigation shows at least one page".into())
        }
        EditorNavigation::Show { stack, .. } if stack.len() > MAX_STACK => {
            Err(format!("a navigation goes at most {MAX_STACK} pages deep"))
        }
        _ => Ok(()),
    }
}

/// What a device shows, as far as navigation goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceNavigation {
    /// The active profile; `None` while locked or without one.
    pub profile: Option<ProfileId>,
    /// The layout the device shows.
    pub layout: Option<LayoutId>,
    /// The pages it went through, the page shown last.
    pub stack: Vec<PageId>,
    pub connected: bool,
    /// The device shows the session's profile and layout, so it shows the
    /// working layout; otherwise the preview is suspended.
    #[serde(default)]
    pub in_scope: bool,
    /// Grows with every state the agent reports for the session (answers
    /// and events alike): of an answer and an event that cross each
    /// other, the higher one is newer.
    #[serde(default)]
    pub seq: u64,
}

impl DeviceNavigation {
    /// The page shown.
    pub fn page(&self) -> Option<PageId> {
        self.stack.last().copied()
    }
}

/// Everything of a layout but its pages: the page order names every page
/// of the revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutHeader {
    pub name: String,
    pub home_page: PageId,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub explicit_tree: bool,
    pub order: Vec<PageId>,
}

/// One part of a revision: the header (first part only) and pages that
/// are new or changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<LayoutHeader>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<LayoutPage>,
}

impl LayoutPatch {
    /// Serialized size of the pages, for the staging limit.
    pub fn size(&self) -> usize {
        self.pages
            .iter()
            .map(|page| serde_json::to_vec(page).map_or(0, |bytes| bytes.len()))
            .sum()
    }
}

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EndReason {
    /// The connection began another session.
    Replaced,
    /// DeckPilot was locked.
    Locked,
    /// The agent is stopping.
    ShuttingDown,
    /// A reason this version does not know.
    #[serde(other)]
    Other,
}

/// Pushed to the connection that owns the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LivePreviewEvent {
    /// The device's navigation changed by itself (not by this session's
    /// `navigateLivePreview`, which answers it).
    Navigated {
        session: u64,
        navigation: DeviceNavigation,
    },
    Ended {
        session: u64,
        reason: EndReason,
    },
}

/// What a simulated device shows (`inspectMockDevice`, mock devices only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MockDeviceInspection {
    pub navigation: DeviceNavigation,
    pub frames: Vec<MockFrame>,
    /// Images written since the device was plugged.
    pub writes: u64,
}

/// One control's image on a simulated device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MockFrame {
    pub control: ControlId,
    /// FNV-1a (64 bit, hex) of the RGB bytes; absent while blank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    /// Images written to it.
    pub writes: u64,
}

/// FNV-1a, 64 bit, as 16 hex digits: a frame's identity for checks.
pub fn frame_hash(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Why a revision could not be split into parts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatchError {
    #[error("page `{0}` is too large to preview live")]
    PageTooLarge(String),
    #[error("the layout has more than {MAX_PAGES} pages")]
    TooManyPages,
}

/// The parts that turn `previous` (the working layout the agent has, or
/// none: everything is sent) into `next`: the header and the pages that
/// differ, at most `budget` bytes of pages per part. Always at least one
/// part, the first with the header.
pub fn patch(
    previous: Option<&DeviceLayout>,
    next: &DeviceLayout,
    budget: usize,
) -> Result<Vec<LayoutPatch>, PatchError> {
    if next.pages.len() > MAX_PAGES {
        return Err(PatchError::TooManyPages);
    }
    let header = LayoutHeader {
        name: next.name.clone(),
        home_page: next.home_page,
        explicit_tree: next.explicit_tree,
        order: next.pages.iter().map(|page| page.id).collect(),
    };
    let mut parts = Vec::new();
    let mut current = LayoutPatch {
        header: Some(header),
        pages: Vec::new(),
    };
    let mut used = 0;
    for page in &next.pages {
        if previous.and_then(|layout| layout.page(page.id)) == Some(page) {
            continue;
        }
        let size = serde_json::to_vec(page).map_or(usize::MAX, |bytes| bytes.len());
        if size > budget {
            return Err(PatchError::PageTooLarge(page.name.clone()));
        }
        if used + size > budget && !current.pages.is_empty() {
            parts.push(std::mem::take(&mut current));
            used = 0;
        }
        current.pages.push(page.clone());
        used += size;
    }
    parts.push(current);
    Ok(parts)
}

/// `base` with the parts of one revision applied: the header's fields and
/// page order, each page taken from the parts or else from `base`. Fails
/// when a part's header is missing or repeated, or a page of the order is
/// in neither.
pub fn apply(base: &DeviceLayout, parts: &[LayoutPatch]) -> Result<DeviceLayout, String> {
    let mut header = None;
    for part in parts {
        if let Some(found) = &part.header {
            if header.is_some() {
                return Err("a revision has one header".into());
            }
            header = Some(found);
        }
    }
    let header = header.ok_or("a revision needs a header")?;
    if header.order.len() > MAX_PAGES {
        return Err(format!("a layout has at most {MAX_PAGES} pages"));
    }
    let mut pages = Vec::with_capacity(header.order.len());
    for id in &header.order {
        if pages.iter().any(|page: &LayoutPage| page.id == *id) {
            return Err(format!("page `{id}` is listed twice"));
        }
        let page = parts
            .iter()
            .rev()
            .flat_map(|part| part.pages.iter().rev())
            .find(|page| page.id == *id)
            .or_else(|| base.page(*id))
            .ok_or_else(|| format!("page `{id}` was not sent"))?;
        pages.push(page.clone());
    }
    if !pages.iter().any(|page| page.id == header.home_page) {
        return Err("the home page does not exist".into());
    }
    Ok(DeviceLayout {
        id: base.id,
        name: header.name.clone(),
        model: base.model.clone(),
        home_page: header.home_page,
        pages,
        explicit_tree: header.explicit_tree,
        recovered: base.recovered.clone(),
    })
}

/// The revisions of one session: only newer ones are taken, and the parts
/// of a revision are collected until its last part.
#[derive(Debug, Default)]
pub struct RevisionGate {
    applied: u64,
    staging: Option<(u64, Vec<LayoutPatch>, usize)>,
}

impl RevisionGate {
    /// The revision applied last (0: none yet).
    pub fn applied(&self) -> u64 {
        self.applied
    }

    /// Takes one part of `revision`: the parts of the revision once `last`
    /// arrived (to apply, then [`RevisionGate::commit`]), `None` while more
    /// are expected. Errors are reasons of [`reason`]: a revision not newer
    /// than the applied one (or an older one than the revision being
    /// collected), or more than [`MAX_STAGED_BYTES`].
    pub fn accept(
        &mut self,
        revision: u64,
        part: LayoutPatch,
        last: bool,
    ) -> Result<Option<Vec<LayoutPatch>>, &'static str> {
        if revision <= self.applied {
            return Err(reason::STALE_REVISION);
        }
        let size = part.size();
        let staged = match self.staging.take() {
            Some((staged, parts, bytes)) if staged == revision => (staged, parts, bytes),
            Some((staged, parts, bytes)) if staged > revision => {
                self.staging = Some((staged, parts, bytes));
                return Err(reason::STALE_REVISION);
            }
            // A newer revision drops what an older one collected.
            _ => (revision, Vec::new(), 0),
        };
        let (_, mut parts, bytes) = staged;
        if bytes + size > MAX_STAGED_BYTES {
            return Err(reason::TOO_LARGE);
        }
        parts.push(part);
        if last {
            Ok(Some(parts))
        } else {
            self.staging = Some((revision, parts, bytes + size));
            Ok(None)
        }
    }

    /// `revision` was applied.
    pub fn commit(&mut self, revision: u64) {
        self.applied = self.applied.max(revision);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::{ActionInstanceId, BindingTarget, ControlBinding, Trigger};
    use deckpilot_device::DeviceModelId;

    fn layout(pages: usize) -> DeviceLayout {
        let mut pages: Vec<LayoutPage> = (0..pages)
            .map(|i| LayoutPage::new(format!("Page {i}")))
            .collect();
        for page in &mut pages {
            for index in 0..15 {
                page.bindings.push(ControlBinding::new(
                    ControlId::Key { index },
                    Trigger::Press,
                    BindingTarget::action(ActionInstanceId::new()),
                ));
            }
        }
        DeviceLayout {
            id: LayoutId::new(),
            name: "Main".into(),
            model: DeviceModelId::new("elgato.stream-deck-mk2"),
            home_page: pages[0].id,
            pages,
            explicit_tree: true,
            recovered: Vec::new(),
        }
    }

    #[test]
    fn a_patch_sends_only_changed_pages_and_applies_back() {
        let before = layout(3);
        let mut after = before.clone();
        after.pages[1].name = "Renamed".into();
        let parts = patch(Some(&before), &after, PART_BUDGET).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].pages.len(), 1);
        assert_eq!(parts[0].pages[0].name, "Renamed");
        assert_eq!(apply(&before, &parts).unwrap(), after);
        // Unchanged: only the header.
        let same = patch(Some(&after), &after, PART_BUDGET).unwrap();
        assert_eq!(same[0].pages.len(), 0);
        assert_eq!(apply(&after, &same).unwrap(), after);
    }

    #[test]
    fn a_first_revision_sends_everything_in_parts_under_the_budget() {
        let big = layout(60);
        let parts = patch(None, &big, PART_BUDGET).unwrap();
        assert!(parts.len() > 1, "{} parts", parts.len());
        assert!(parts[0].header.is_some());
        assert!(parts[1..].iter().all(|part| part.header.is_none()));
        for part in &parts {
            let size = serde_json::to_vec(part).unwrap().len();
            assert!(size < crate::framing::MAX_MESSAGE_BYTES - 1024, "{size}");
        }
        // Applied on anything with the id and model: every page was sent.
        let mut empty = big.clone();
        empty.pages.truncate(1);
        assert_eq!(apply(&empty, &parts).unwrap(), big);
    }

    #[test]
    fn removed_and_reordered_pages_follow_the_order() {
        let before = layout(3);
        let mut after = before.clone();
        after.pages.remove(2);
        after.pages.swap(0, 1);
        let parts = patch(Some(&before), &after, PART_BUDGET).unwrap();
        assert_eq!(parts[0].pages.len(), 0);
        assert_eq!(apply(&before, &parts).unwrap(), after);
    }

    #[test]
    fn applying_refuses_what_does_not_add_up() {
        let base = layout(2);
        let missing = LayoutPatch {
            header: Some(LayoutHeader {
                name: "x".into(),
                home_page: base.home_page,
                explicit_tree: true,
                order: vec![base.home_page, PageId::new()],
            }),
            pages: Vec::new(),
        };
        assert!(apply(&base, &[missing]).is_err());
        assert!(apply(&base, &[LayoutPatch::default()]).is_err());
        let no_home = LayoutPatch {
            header: Some(LayoutHeader {
                name: "x".into(),
                home_page: PageId::new(),
                explicit_tree: true,
                order: vec![base.home_page],
            }),
            pages: Vec::new(),
        };
        assert!(apply(&base, &[no_home]).is_err());
    }

    #[test]
    fn an_oversized_page_is_refused() {
        let mut big = layout(1);
        big.pages[0].name = "x".repeat(PART_BUDGET + 1);
        assert!(matches!(
            patch(None, &big, PART_BUDGET),
            Err(PatchError::PageTooLarge(_))
        ));
    }

    fn part() -> LayoutPatch {
        LayoutPatch::default()
    }

    #[test]
    fn revisions_only_go_forward_and_parts_are_collected() {
        let mut gate = RevisionGate::default();
        assert_eq!(gate.accept(2, part(), true).unwrap().unwrap().len(), 1);
        gate.commit(2);
        assert_eq!(gate.accept(2, part(), true), Err(reason::STALE_REVISION));
        assert_eq!(gate.accept(1, part(), true), Err(reason::STALE_REVISION));
        // Three parts of revision 5, applied with the last.
        assert_eq!(gate.accept(5, part(), false), Ok(None));
        assert_eq!(gate.accept(5, part(), false), Ok(None));
        // An older revision while 5 is collected is late.
        assert_eq!(gate.accept(4, part(), true), Err(reason::STALE_REVISION));
        assert_eq!(gate.accept(5, part(), true).unwrap().unwrap().len(), 3);
        gate.commit(5);
        assert_eq!(gate.applied(), 5);
        // A newer revision drops what an older one collected (gaps are fine).
        assert_eq!(gate.accept(6, part(), false), Ok(None));
        assert_eq!(gate.accept(9, part(), true).unwrap().unwrap().len(), 1);
    }

    #[test]
    fn staging_is_bounded() {
        let mut gate = RevisionGate::default();
        let mut page = LayoutPage::new("x".repeat(PART_BUDGET - 200));
        page.bindings.clear();
        let big = LayoutPatch {
            header: None,
            pages: vec![page],
        };
        let mut refused = false;
        for _ in 0..(MAX_STAGED_BYTES / PART_BUDGET + 2) {
            if gate.accept(1, big.clone(), false) == Err(reason::TOO_LARGE) {
                refused = true;
                break;
            }
        }
        assert!(refused);
    }

    #[test]
    fn shapes_on_the_wire() {
        let show = EditorNavigation::Show {
            layout: LayoutId::new(),
            stack: vec![PageId::new()],
        };
        let json = serde_json::to_value(&show).unwrap();
        assert_eq!(json["type"], "show");
        assert_eq!(
            serde_json::from_value::<EditorNavigation>(json).unwrap(),
            show
        );
        assert_eq!(
            serde_json::to_value(EditorNavigation::LayoutBack).unwrap(),
            serde_json::json!({"type": "layoutBack"})
        );
        let event = LivePreviewEvent::Ended {
            session: 3,
            reason: EndReason::Locked,
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({"type": "ended", "session": 3, "reason": "locked"})
        );
        let future: EndReason = serde_json::from_str("\"somethingNew\"").unwrap();
        assert_eq!(future, EndReason::Other);
        assert_eq!(frame_hash(b""), "cbf29ce484222325");
    }
}
