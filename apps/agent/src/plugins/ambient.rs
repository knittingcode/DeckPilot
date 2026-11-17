//! The two capabilities of plugin API 1.1 that reach nothing of the
//! user's: the time (`time.now`) and random bytes (`random.bytes`). The
//! broker allows both for every enabled plugin and rate limits them; here
//! they are carried out.
//!
//! - The time is the system's wall clock and the local UTC offset at that
//!   instant. No time zone name leaves the agent.
//! - Random bytes come from the operating system's CSPRNG, fresh for every
//!   call. The agent keeps no generator a plugin could observe or steer,
//!   and never returns more than the ABI's bound whatever it is asked for.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Offset as _;
use deckpilot_plugin_abi::capability::{RandomReply, Timestamp};
use deckpilot_plugin_abi::limits::MAX_RANDOM_BYTES;
use deckpilot_plugin_abi::{CallErrorCode, CallResult};

/// The answer to `time.now`.
pub(super) fn now() -> CallResult {
    let at = SystemTime::now();
    let Ok(since_epoch) = at.duration_since(UNIX_EPOCH) else {
        return CallResult::error(
            CallErrorCode::Unavailable,
            "the system clock is set before 1970",
        );
    };
    let local: chrono::DateTime<chrono::Local> = at.into();
    let offset_minutes = local.offset().fix().local_minus_utc() / 60;
    let unix_ms = u64::try_from(since_epoch.as_millis()).unwrap_or(u64::MAX);
    reply(&Timestamp::new(unix_ms, offset_minutes))
}

/// The answer to `random.bytes` for `length` bytes.
pub(super) fn random(length: usize) -> CallResult {
    if length == 0 || length > MAX_RANDOM_BYTES {
        // The broker never allows it; an `Allowed` built elsewhere gets
        // no more than the bound either.
        return CallResult::error(CallErrorCode::BadRequest, "length exceeds its limit");
    }
    let mut bytes = [0u8; MAX_RANDOM_BYTES];
    let bytes = &mut bytes[..length];
    if getrandom::fill(bytes).is_err() {
        // Never a weaker source instead.
        return CallResult::error(
            CallErrorCode::Unavailable,
            "the system's random source is not available",
        );
    }
    reply(&RandomReply::new(bytes))
}

fn reply<T: serde::Serialize>(value: &T) -> CallResult {
    serde_json::to_value(value).map_or_else(
        |e| CallResult::error(CallErrorCode::Failed, e.to_string()),
        CallResult::Ok,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok<T: serde::de::DeserializeOwned>(result: CallResult) -> T {
        match result {
            CallResult::Ok(value) => serde_json::from_value(value).unwrap(),
            CallResult::Error(e) => panic!("{e:?}"),
        }
    }

    #[test]
    fn the_time_is_the_system_clock_with_the_local_offset() {
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let now: Timestamp = ok(now());
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        assert!((before..=after).contains(&u128::from(now.unix_ms)));
        let local = chrono::Local::now().offset().fix().local_minus_utc() / 60;
        assert_eq!(now.utc_offset_minutes, local);
        assert!(now.utc_offset_minutes.abs() <= 24 * 60);
        // Exactly two members: no zone name, nothing else.
        let CallResult::Ok(value) = super::now() else {
            panic!()
        };
        let members: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(members, ["unixMs", "utcOffsetMinutes"]);
    }

    #[test]
    fn random_bytes_are_fresh_and_bounded() {
        for length in [1, 16, MAX_RANDOM_BYTES] {
            let reply: RandomReply = ok(random(length));
            assert_eq!(reply.decode().unwrap().len(), length);
        }
        let a: RandomReply = ok(random(32));
        let b: RandomReply = ok(random(32));
        assert_ne!(a, b);
        assert_ne!(a.decode().unwrap(), [0u8; 32]);
        for length in [0, MAX_RANDOM_BYTES + 1, usize::MAX] {
            let CallResult::Error(e) = random(length) else {
                panic!("{length}")
            };
            assert_eq!(e.code, CallErrorCode::BadRequest);
        }
    }
}
