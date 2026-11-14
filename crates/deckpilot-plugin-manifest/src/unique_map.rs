//! Deserializes a JSON object into a map, refusing duplicate keys.
//!
//! `serde_json` keeps the last of two equal keys without a word, and other
//! JSON parsers keep the first. For `keyIcons` and `errorCodes` the two
//! readings would differ, so a duplicate is an error.

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use serde::de::{Deserialize, Deserializer, Error, MapAccess, Visitor};

pub(crate) fn deserialize<'de, D, V>(
    deserializer: D,
) -> Result<Option<BTreeMap<String, V>>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct UniqueVisitor<V>(PhantomData<V>);

    impl<'de, V: Deserialize<'de>> Visitor<'de> for UniqueVisitor<V> {
        type Value = BTreeMap<String, V>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an object with unique keys")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
            let mut map = BTreeMap::new();
            while let Some((key, value)) = access.next_entry::<String, V>()? {
                if map.contains_key(&key) {
                    return Err(A::Error::custom(format_args!("duplicate key `{key}`")));
                }
                map.insert(key, value);
            }
            Ok(map)
        }
    }

    deserializer
        .deserialize_map(UniqueVisitor(PhantomData))
        .map(Some)
}
