//! Pure helpers: no capability calls, no clock, no randomness (a guest on
//! `wasm32-unknown-unknown` has neither, and ABI 1 offers no capability
//! for them).

/// Seconds of 0001-01-01T00:00:00Z and 9999-12-31T23:59:59Z: the range of
/// four-digit years.
pub const MIN_SECONDS: i64 = -62_135_596_800;
pub const MAX_SECONDS: i64 = 253_402_300_799;
/// A bare number at least this large (in magnitude) is read as
/// milliseconds by `from: auto`: as seconds it would be after the year 5138.
pub const AUTO_MILLIS_FROM: i64 = 100_000_000_000;

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's
/// `days_from_civil`).
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day count since 1970-01-01 (`civil_from_days`).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `YYYY-MM-DDTHH:MM:SS[.mmm]Z` of a millisecond time stamp (UTC).
pub fn format_rfc3339(millis: i64) -> String {
    let seconds = millis.div_euclid(1000);
    let ms = millis.rem_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (h, m, s) = (rest / 3600, rest % 3600 / 60, rest % 60);
    if ms == 0 {
        format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
    } else {
        format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{ms:03}Z")
    }
}

/// Parses an RFC 3339 date-time (`2026-11-15T09:30:00Z`,
/// `2026-11-15 09:30:00.250+01:00`; `T` or a space, `Z` or an offset;
/// fractions beyond milliseconds are cut) into milliseconds since the
/// epoch.
pub fn parse_rfc3339(text: &str) -> Result<i64, String> {
    let bad = || format!("`{}` is not an RFC 3339 date-time", cut(text, 40));
    let b = text.as_bytes();
    if b.len() < 20 {
        return Err(bad());
    }
    let num = |range: std::ops::Range<usize>| -> Result<i64, String> {
        let part = text.get(range).ok_or_else(bad)?;
        if part.bytes().all(|c| c.is_ascii_digit()) {
            part.parse().map_err(|_| bad())
        } else {
            Err(bad())
        }
    };
    let sep_ok = b[4] == b'-'
        && b[7] == b'-'
        && matches!(b[10], b'T' | b't' | b' ')
        && b[13] == b':'
        && b[16] == b':';
    if !sep_ok {
        return Err(bad());
    }
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut i = 19;
    let mut millis = 0;
    if b[i] == b'.' {
        let start = i + 1;
        let mut end = start;
        while end < b.len() && b[end].is_ascii_digit() {
            end += 1;
        }
        if end == start {
            return Err(bad());
        }
        let digits = &text[start..end.min(start + 3)];
        millis = digits.parse::<i64>().map_err(|_| bad())? * 10_i64.pow(3 - digits.len() as u32);
        i = end;
    }
    let offset_minutes = match &text[i..] {
        "Z" | "z" => 0,
        zone if zone.len() == 6 && matches!(zone.as_bytes()[0], b'+' | b'-') => {
            if zone.as_bytes()[3] != b':' {
                return Err(bad());
            }
            let (h, m) = (num(i + 1..i + 3)?, num(i + 4..i + 6)?);
            if h > 23 || m > 59 {
                return Err(bad());
            }
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            sign * (h * 60 + m)
        }
        _ => return Err(bad()),
    };
    let month_ok = (1..=12).contains(&month);
    if !month_ok
        || day < 1
        || day > i64::from(days_in_month(year, month as u32))
        || hour > 23
        || minute > 59
        || second > 60
    {
        return Err(format!("`{}` is not a valid date or time", cut(text, 40)));
    }
    // A leap second is read as the last second of its minute.
    let second = second.min(59);
    let days = days_from_civil(year, month as u32, day as u32);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_minutes * 60;
    Ok(seconds * 1000 + millis)
}

/// Unit of a time stamp text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    Seconds,
    Millis,
    Rfc3339,
}

/// Reads `text` as `from` (`auto`, `seconds`, `milliseconds`, `rfc3339`)
/// and returns milliseconds since the epoch and what it was read as.
pub fn parse_timestamp(text: &str, from: &str) -> Result<(i64, TimeUnit), String> {
    let text = text.trim();
    let integer = || {
        text.parse::<i64>()
            .map_err(|_| format!("`{}` is not a whole number", cut(text, 40)))
    };
    let (millis, unit) = match from {
        "seconds" => (integer()?.checked_mul(1000), TimeUnit::Seconds),
        "milliseconds" => (Some(integer()?), TimeUnit::Millis),
        "rfc3339" => (Some(parse_rfc3339(text)?), TimeUnit::Rfc3339),
        _ if text.starts_with(['-', '+']) || text.bytes().all(|c| c.is_ascii_digit()) => {
            let n = integer()?;
            if n.unsigned_abs() >= AUTO_MILLIS_FROM.unsigned_abs() {
                (Some(n), TimeUnit::Millis)
            } else {
                (Some(n * 1000), TimeUnit::Seconds)
            }
        }
        _ => (Some(parse_rfc3339(text)?), TimeUnit::Rfc3339),
    };
    let millis = millis.ok_or_else(|| "the time stamp is out of range".to_owned())?;
    if !(MIN_SECONDS * 1000..=MAX_SECONDS * 1000 + 999).contains(&millis) {
        return Err("the time stamp is outside the years 0001 to 9999".into());
    }
    Ok((millis, unit))
}

/// SHA-1 (FIPS 180-4). Only for name-based UUIDs (RFC 9562 version 5),
/// never for security.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// The RFC 9562 namespace ids for `dns`, `url`, `oid` and `x500`.
pub fn namespace(name: &str) -> Option<[u8; 16]> {
    let last = match name {
        "dns" => 0x10,
        "url" => 0x11,
        "oid" => 0x12,
        "x500" => 0x14,
        _ => return None,
    };
    Some([
        0x6b, 0xa7, 0xb8, last, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30,
        0xc8,
    ])
}

/// A name-based UUID, version 5 (SHA-1), in lower-case hyphenated form.
pub fn uuid_v5(namespace: [u8; 16], name: &str) -> String {
    let mut input = namespace.to_vec();
    input.extend_from_slice(name.as_bytes());
    let hash = sha1(&input);
    let mut b = [0u8; 16];
    b.copy_from_slice(&hash[..16]);
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Percent-encodes a query or path component (RFC 3986 unreserved
/// characters stay).
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `owner/name` as GitHub allows them: owner 1-39 letters, digits or
/// hyphens; name 1-100 letters, digits, `.`, `_` or `-`, not `.` or `..`.
pub fn is_repo(text: &str) -> bool {
    let Some((owner, name)) = text.split_once('/') else {
        return false;
    };
    (1..=39).contains(&owner.len())
        && owner
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        && (1..=100).contains(&name.len())
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && name != "."
        && name != ".."
}

/// A git branch name, roughly as `git check-ref-format --branch` accepts
/// it: no control characters, spaces or `~^:?*[\`, no `..`, no leading
/// `-` or `/`.
pub fn is_branch(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 100
        && !text.starts_with(['-', '/'])
        && !text.ends_with(['/', '.'])
        && !text.contains("..")
        && !text.contains("//")
        && !text.contains("@{")
        && !text.ends_with(".lock")
        && text.chars().all(|c| {
            !c.is_control() && !matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
}

/// At most `n` characters of `text` (for messages).
pub fn cut(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}
