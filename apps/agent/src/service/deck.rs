//! What actions ask the agent to do to the deck (`deckpilot_core::DeckRequest`)
//! and the pages devices return to.
//!
//! - **Profile switches.** The device runtime applies the page requests of
//!   an action and leaves the rest in its result. A *Switch profile* is
//!   applied here, on the service task, through the same path as the
//!   `setActiveProfile` request, so devices, gestures, key states and the
//!   app follow at once. A profile that does not exist turns the result
//!   into a failure with a code.
//! - **Pages.** The top-level page each device shows per profile is saved
//!   in the `settings` table whenever it changed, and handed back to the
//!   runtime after a restart. It is runtime state of this computer: not
//!   part of a profile, never exported.

use deckpilot_core::{ActionResult, DeckRequest, PageId, ProfileId, ProfileTarget};
use deckpilot_device::DeviceId;
use deckpilot_ipc::ProfileInfo;
use deckpilot_storage::ConfigStore;
use tracing::warn;

use super::Agent;

/// The setting the pages are saved in.
const PAGES_SETTING: &str = "navigation.pages";

/// The profile to switch to does not exist (any more).
const PROFILE_NOT_FOUND: &str = "profile-not-found";

/// The brightness could not be saved.
const BRIGHTNESS_NOT_SAVED: &str = "brightness-not-saved";

/// The brightness after a request: `set` as it is, else `current` moved by
/// `step`; always within 0-100.
fn brightness_after(current: u8, set: Option<u8>, step: i8) -> u8 {
    match set {
        Some(level) => level.min(100),
        None => u8::try_from((i16::from(current) + i16::from(step)).clamp(0, 100)).unwrap_or(100),
    }
}

/// One entry of [`PAGES_SETTING`], a JSON list: a profile, a device and
/// the top-level page the device shows in that profile.
type SavedPage = (ProfileId, DeviceId, PageId);

/// The profile `target` names among `profiles`, seen from `active`.
/// `Ok(None)`: there is nowhere to go (one profile, or the end of the list
/// without wraparound).
fn pick(
    profiles: &[ProfileInfo],
    active: Option<ProfileId>,
    target: ProfileTarget,
    wrap_around: bool,
) -> Result<Option<&ProfileInfo>, ()> {
    let forward = match target {
        ProfileTarget::Profile(id) => {
            return profiles.iter().find(|p| p.id == id).map(Some).ok_or(());
        }
        ProfileTarget::Next => true,
        ProfileTarget::Previous => false,
    };
    let count = profiles.len();
    let Some(index) = active.and_then(|id| profiles.iter().position(|p| p.id == id)) else {
        // No profile is active: the first or the last one.
        return Ok(if forward {
            profiles.first()
        } else {
            profiles.last()
        });
    };
    if count < 2 {
        return Ok(None);
    }
    let next = match (forward, index) {
        (true, last) if last + 1 == count => wrap_around.then_some(0),
        (true, index) => Some(index + 1),
        (false, 0) => wrap_around.then_some(count - 1),
        (false, index) => Some(index - 1),
    };
    Ok(next.and_then(|index| profiles.get(index)))
}

impl Agent {
    /// Applies the request the device runtime left in `result` (a profile
    /// switch) and takes it out. A request that cannot be applied makes the
    /// result a failure.
    pub(super) fn apply_deck_request(&mut self, result: &mut ActionResult) {
        // A page request belongs to a device and was applied by the
        // runtime; one that is left had no device.
        let (target, wrap_around) = match result.request.take().map(|request| *request) {
            Some(DeckRequest::SwitchProfile {
                target,
                wrap_around,
            }) => (target, wrap_around),
            Some(DeckRequest::Brightness { set, step }) => {
                self.apply_brightness(set, step, result);
                return;
            }
            _ => return,
        };
        let profiles = self.profiles();
        let active = self.runtime.profile().map(|profile| profile.id);
        match pick(&profiles, active, target, wrap_around) {
            Ok(Some(profile)) if Some(profile.id) == active => {
                result.message = format!("profile {} is active", profile.name);
            }
            Ok(Some(profile)) => {
                let (id, name) = (profile.id, profile.name.clone());
                if let Err(error) = self.set_active_profile(id) {
                    *result = ActionResult::error(PROFILE_NOT_FOUND, error.message);
                    return;
                }
                result.message = format!("profile {name}");
                result.data.insert("profileId".into(), id.to_string());
                result.data.insert("profileName".into(), name);
            }
            Ok(None) => {
                result.message = "there is no other profile".into();
            }
            Err(()) => {
                *result = ActionResult::error(
                    PROFILE_NOT_FOUND,
                    "the profile to switch to does not exist any more",
                );
            }
        }
    }

    /// Sets the display brightness as the `setBrightness` request does: the
    /// setting is saved and sent to every device that has a brightness.
    fn apply_brightness(&mut self, set: Option<u8>, step: i8, result: &mut ActionResult) {
        let level = brightness_after(self.brightness, set, step);
        if level != self.brightness
            && let Err(error) = self.set_brightness(level)
        {
            *result = ActionResult::error(BRIGHTNESS_NOT_SAVED, error.message);
            return;
        }
        result.message = format!("brightness {level} %");
        result.data.insert("brightness".into(), level.to_string());
    }

    /// Hands the pages saved by an earlier run to the runtime. Called after
    /// unlocking, before the active profile is loaded. Pages of profiles
    /// that are gone are dropped; a page that is gone is ignored by the
    /// runtime when its device connects.
    pub(super) fn restore_pages(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let saved = match store.setting(PAGES_SETTING) {
            Ok(Some(saved)) => saved,
            Ok(None) => return,
            Err(error) => {
                warn!(%error, "cannot read the saved pages");
                return;
            }
        };
        // An unreadable value is ignored: devices start on their home page.
        let Ok(pages) = serde_json::from_str::<Vec<SavedPage>>(&saved) else {
            warn!("the saved pages are unreadable and were ignored");
            return;
        };
        let known: Vec<ProfileId> = self.profiles().into_iter().map(|p| p.id).collect();
        self.runtime.restore_pages(
            pages
                .into_iter()
                .filter(|(profile, _, _)| known.contains(profile)),
        );
        self.saved_pages = Some(saved);
    }

    /// Saves the page each device shows per profile, if it changed.
    pub(super) fn persist_pages(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let pages: Vec<SavedPage> = self.runtime.remembered_pages();
        let Ok(value) = serde_json::to_string(&pages) else {
            return;
        };
        if self.saved_pages.as_deref() == Some(value.as_str())
            || (self.saved_pages.is_none() && pages.is_empty())
        {
            return;
        }
        match store.set_setting(PAGES_SETTING, &value) {
            Ok(()) => self.saved_pages = Some(value),
            Err(error) => warn!(%error, "cannot save the pages devices show"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profiles(names: &[&str]) -> Vec<ProfileInfo> {
        names
            .iter()
            .map(|name| ProfileInfo {
                id: ProfileId::new(),
                name: (*name).to_owned(),
            })
            .collect()
    }

    fn name(picked: Result<Option<&ProfileInfo>, ()>) -> Option<&str> {
        picked.unwrap().map(|profile| profile.name.as_str())
    }

    #[test]
    fn next_and_previous_follow_the_list_and_wrap_when_asked() {
        let list = profiles(&["A", "B", "C"]);
        let at = |index: usize| Some(list[index].id);
        assert_eq!(
            name(pick(&list, at(0), ProfileTarget::Next, true)),
            Some("B")
        );
        assert_eq!(
            name(pick(&list, at(2), ProfileTarget::Next, true)),
            Some("A")
        );
        assert_eq!(name(pick(&list, at(2), ProfileTarget::Next, false)), None);
        assert_eq!(
            name(pick(&list, at(0), ProfileTarget::Previous, true)),
            Some("C")
        );
        assert_eq!(
            name(pick(&list, at(0), ProfileTarget::Previous, false)),
            None
        );
        assert_eq!(
            name(pick(&list, at(1), ProfileTarget::Previous, false)),
            Some("A")
        );
    }

    #[test]
    fn a_single_profile_has_nowhere_to_go() {
        let list = profiles(&["Only"]);
        let active = Some(list[0].id);
        assert_eq!(name(pick(&list, active, ProfileTarget::Next, true)), None);
        assert_eq!(
            name(pick(&list, active, ProfileTarget::Previous, true)),
            None
        );
        // Without an active profile, next is the first one.
        assert_eq!(
            name(pick(&list, None, ProfileTarget::Next, true)),
            Some("Only")
        );
        assert_eq!(name(pick(&[], None, ProfileTarget::Next, true)), None);
    }

    #[test]
    fn brightness_is_set_or_stepped_within_its_range() {
        assert_eq!(brightness_after(60, Some(25), 0), 25);
        assert_eq!(brightness_after(60, Some(200), 0), 100);
        assert_eq!(brightness_after(60, None, 10), 70);
        assert_eq!(brightness_after(95, None, 10), 100);
        assert_eq!(brightness_after(5, None, -10), 0);
        assert_eq!(brightness_after(60, None, -25), 35);
    }

    #[test]
    fn a_specific_profile_must_exist() {
        let list = profiles(&["A", "B"]);
        let b = list[1].id;
        assert_eq!(
            name(pick(
                &list,
                Some(list[0].id),
                ProfileTarget::Profile(b),
                false
            )),
            Some("B")
        );
        assert!(pick(&list, None, ProfileTarget::Profile(ProfileId::new()), true).is_err());
    }
}
