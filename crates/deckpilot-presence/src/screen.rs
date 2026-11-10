//! Whether the user's screen is locked, for "lock DeckPilot when the screen
//! locks". Polled by the agent; only platforms where the state can be read
//! reliably report a monitor.
//!
//! | Platform | Source                                                          |
//! | -------- | --------------------------------------------------------------- |
//! | macOS    | `CGSessionCopyCurrentDictionary`: `CGSSessionScreenIsLocked`      |
//! | Linux    | systemd-logind: `LockedHint` of the caller's session, which GNOME, KDE Plasma and other lockers set |
//! | Windows  | not implemented                                                 |

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

/// Reads the lock state of the current user session.
pub trait ScreenLockMonitor: Send + Sync + fmt::Debug {
    /// `Some(true)` while the screen is locked; `None` if the state cannot
    /// be read right now.
    fn is_locked(&self) -> Option<bool>;
}

/// The monitor of this platform, or `None` where the screen lock cannot be
/// detected (then the setting is not offered).
pub fn screen_lock_monitor() -> Option<Box<dyn ScreenLockMonitor>> {
    #[cfg(target_os = "macos")]
    {
        macos::CgSession::probe().map(|m| Box::new(m) as Box<dyn ScreenLockMonitor>)
    }
    #[cfg(target_os = "linux")]
    {
        linux::Logind::probe().map(|m| Box::new(m) as Box<dyn ScreenLockMonitor>)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// A monitor whose state tests set.
#[derive(Debug, Default)]
pub struct ScriptedScreenLock(AtomicBool);

impl ScriptedScreenLock {
    pub fn set_locked(&self, locked: bool) {
        self.0.store(locked, Ordering::SeqCst);
    }
}

impl ScreenLockMonitor for ScriptedScreenLock {
    fn is_locked(&self) -> Option<bool> {
        Some(self.0.load(Ordering::SeqCst))
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    use objc2_core_foundation::{CFBoolean, CFString, CFType};
    use objc2_core_graphics::CGSessionCopyCurrentDictionary;

    use super::ScreenLockMonitor;

    /// The session dictionary of the window server. It exists for processes
    /// in the user's graphical session (the agent started at login); the
    /// key `CGSSessionScreenIsLocked` is present and true while locked.
    #[derive(Debug)]
    pub struct CgSession;

    impl CgSession {
        pub fn probe() -> Option<Self> {
            CGSessionCopyCurrentDictionary().map(|_| Self)
        }
    }

    impl ScreenLockMonitor for CgSession {
        fn is_locked(&self) -> Option<bool> {
            let session = CGSessionCopyCurrentDictionary()?;
            let key = CFString::from_static_str("CGSSessionScreenIsLocked");
            // SAFETY: `session` and `key` are valid CF objects for the whole
            // block. The dictionary uses CF objects as keys, so passing the
            // string's pointer is the documented lookup; the returned value
            // is owned by `session`, which outlives the reference.
            unsafe {
                let value = session.value((&raw const *key).cast());
                if value.is_null() {
                    return Some(false);
                }
                let value = &*value.cast::<CFType>();
                Some(
                    value
                        .downcast_ref::<CFBoolean>()
                        .is_some_and(CFBoolean::value),
                )
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use zbus::blocking::{Connection, Proxy};
    use zbus::proxy::CacheProperties;

    use super::ScreenLockMonitor;

    /// The `LockedHint` property of the caller's logind session.
    pub struct Logind {
        proxy: Proxy<'static>,
    }

    impl std::fmt::Debug for Logind {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Logind")
        }
    }

    impl Logind {
        /// Available when the system bus answers for the caller's session
        /// (`/org/freedesktop/login1/session/auto`).
        pub fn probe() -> Option<Self> {
            let connection = Connection::system().ok()?;
            let proxy = zbus::blocking::proxy::Builder::new(&connection)
                .destination("org.freedesktop.login1")
                .ok()?
                .path("/org/freedesktop/login1/session/auto")
                .ok()?
                .interface("org.freedesktop.login1.Session")
                .ok()?
                .cache_properties(CacheProperties::No)
                .build()
                .ok()?;
            let monitor = Self { proxy };
            monitor.is_locked().map(|_| monitor)
        }
    }

    impl ScreenLockMonitor for Logind {
        fn is_locked(&self) -> Option<bool> {
            self.proxy.get_property::<bool>("LockedHint").ok()
        }
    }
}
