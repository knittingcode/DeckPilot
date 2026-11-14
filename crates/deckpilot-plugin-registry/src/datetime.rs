//! RFC 3339 date-times (`format: date-time` in the schemas), parsed
//! strictly and always held in UTC.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serializer};

pub(crate) fn parse(text: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| format!("`{text}` is not an RFC 3339 date-time: {e}"))
}

pub(crate) fn format(time: &DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub(crate) fn serialize<S: Serializer>(time: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format(time))
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
    let text = String::deserialize(d)?;
    parse(&text).map_err(serde::de::Error::custom)
}

pub(crate) mod option {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    #[allow(clippy::ref_option)]
    pub(crate) fn serialize<S: Serializer>(
        time: &Option<DateTime<Utc>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match time {
            Some(time) => s.serialize_some(&super::format(time)),
            None => s.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<DateTime<Utc>>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|text| super::parse(&text).map_err(serde::de::Error::custom))
            .transpose()
    }
}
