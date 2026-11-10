//! Asks the operating system to verify that the logged-in user is present.
//!
//! DeckPilot uses this in system-managed key mode, where no DeckPilot
//! password exists: to unlock after an automatic lock and to confirm
//! sensitive operations. Verification is done entirely by the platform;
//! DeckPilot never sees fingerprints or the login password.
//!
//! | Platform | Method                                                     |
//! | -------- | ---------------------------------------------------------- |
//! | macOS    | `LocalAuthentication`: Touch ID, or the login password      |
//! | others   | not available ([`Unavailable`]); nothing is claimed         |

use std::fmt;

/// How the platform verifies the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Touch ID, with the login password as alternative.
    TouchId,
    /// The login password (no biometric sensor, or none enrolled).
    DevicePassword,
}

#[derive(Debug, thiserror::Error)]
pub enum PresenceError {
    #[error("authentication was cancelled")]
    Cancelled,
    #[error("authentication failed")]
    Failed,
    #[error("system authentication is not available: {0}")]
    Unavailable(String),
}

/// A user verification provided by the operating system.
pub trait PresenceCheck: Send + Sync + fmt::Debug {
    /// The method that would be used now, or `None` if verification is not
    /// possible on this computer.
    fn method(&self) -> Option<Method>;
    /// Shows the system prompt and blocks until the user answered. `reason`
    /// completes the sentence "DeckPilot is trying to …".
    fn verify(&self, reason: &str) -> Result<(), PresenceError>;
}

/// The check of this platform.
pub fn platform() -> Box<dyn PresenceCheck> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::LocalAuthentication)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(Unavailable)
    }
}

/// No verification on this platform.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unavailable;

impl PresenceCheck for Unavailable {
    fn method(&self) -> Option<Method> {
        None
    }

    fn verify(&self, _reason: &str) -> Result<(), PresenceError> {
        Err(PresenceError::Unavailable(
            "this platform has no supported verification".into(),
        ))
    }
}

/// A scripted check for tests.
#[derive(Debug)]
pub struct Scripted {
    pub method: Option<Method>,
    /// Whether `verify` succeeds.
    pub succeed: std::sync::atomic::AtomicBool,
    /// Number of prompts shown.
    pub prompts: std::sync::atomic::AtomicUsize,
}

impl Scripted {
    pub fn new(method: Option<Method>, succeed: bool) -> Self {
        Self {
            method,
            succeed: succeed.into(),
            prompts: 0.into(),
        }
    }
}

impl PresenceCheck for Scripted {
    fn method(&self) -> Option<Method> {
        self.method
    }

    fn verify(&self, _reason: &str) -> Result<(), PresenceError> {
        use std::sync::atomic::Ordering;
        if self.method.is_none() {
            return Err(PresenceError::Unavailable("scripted".into()));
        }
        self.prompts.fetch_add(1, Ordering::SeqCst);
        if self.succeed.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(PresenceError::Failed)
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    //! `LAContext` calls. Each context is used for one evaluation only, so
    //! a previous success is never reused.

    use std::sync::mpsc;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LABiometryType, LAContext, LAError, LAPolicy};

    use super::{Method, PresenceCheck, PresenceError};

    /// Touch ID or the login password, whichever the user chooses.
    const POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthentication;

    #[derive(Debug)]
    pub struct LocalAuthentication;

    impl PresenceCheck for LocalAuthentication {
        fn method(&self) -> Option<Method> {
            // SAFETY: creating a context and querying it has no
            // preconditions; the context is released when dropped.
            unsafe {
                let context = LAContext::new();
                context.canEvaluatePolicy_error(POLICY).ok()?;
                Some(if context.biometryType() == LABiometryType::TouchID {
                    Method::TouchId
                } else {
                    Method::DevicePassword
                })
            }
        }

        fn verify(&self, reason: &str) -> Result<(), PresenceError> {
            if reason.is_empty() {
                // An empty reason raises an Objective-C exception.
                return Err(PresenceError::Failed);
            }
            let (sender, receiver) = mpsc::sync_channel::<Result<(), PresenceError>>(1);
            // SAFETY: the context is kept alive by the block until the reply
            // arrives; the block only sends on a channel (it is `Send`), and
            // a non-null error pointer is valid for the duration of the call.
            unsafe {
                let context = LAContext::new();
                context.canEvaluatePolicy_error(POLICY).map_err(|error| {
                    PresenceError::Unavailable(error.localizedDescription().to_string())
                })?;
                let keep_alive = context.clone();
                let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
                    let _context = &keep_alive;
                    let outcome = if success.as_bool() {
                        Ok(())
                    } else if error.is_null() {
                        Err(PresenceError::Failed)
                    } else {
                        let code = (*error).code();
                        if code == LAError::UserCancel.0
                            || code == LAError::SystemCancel.0
                            || code == LAError::AppCancel.0
                        {
                            Err(PresenceError::Cancelled)
                        } else {
                            Err(PresenceError::Failed)
                        }
                    };
                    let _ = sender.try_send(outcome);
                });
                context.evaluatePolicy_localizedReason_reply(
                    POLICY,
                    &NSString::from_str(reason),
                    &reply,
                );
            }
            receiver.recv().unwrap_or(Err(PresenceError::Failed))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_platforms_claim_nothing() {
        assert_eq!(Unavailable.method(), None);
        assert!(Unavailable.verify("test").is_err());
    }

    #[test]
    fn scripted_checks_count_prompts() {
        let check = Scripted::new(Some(Method::TouchId), false);
        assert!(check.verify("test").is_err());
        check
            .succeed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(check.verify("test").is_ok());
        assert_eq!(check.prompts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}

/// Manual check on a real computer: `cargo test -p deckpilot-presence --
/// --ignored --nocapture`. Only queries availability; shows no prompt.
#[cfg(test)]
#[test]
#[ignore = "queries the real platform"]
fn reports_the_platform_method() {
    println!("method: {:?}", platform().method());
}
