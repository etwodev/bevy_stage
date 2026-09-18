//! The normalized value model that all three tag authoring channels produce.

use bevy::prelude::*;
use bevy::platform::collections::HashMap;
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use thiserror::Error;

/// A single value parsed from artist-authored metadata.
///
/// Mirrors the JSON model glTF `extras` are written in, so anything a DCC can
/// export round-trips. [`BTreeMap`] rather than a hash map so that iteration
/// order is deterministic, which keeps error messages and test assertions stable.
#[derive(Clone, Debug, PartialEq, Reflect, Default)]
pub enum TagValue {
    /// An explicit null, or a property present with no value.
    #[default]
    Null,
    /// A boolean.
    Bool(bool),
    /// Any number. glTF has no integer/float distinction in `extras`, so
    /// everything is widened to `f64` and narrowed on access.
    Number(f64),
    /// A string.
    String(String),
    /// An ordered list.
    List(Vec<TagValue>),
    /// A nested object.
    Map(BTreeMap<String, TagValue>),
}

impl TagValue {
    /// The value as a bool, accepting the string forms `"true"`/`"false"` that
    /// DCCs often produce for what the artist intended as a checkbox.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => Some(true),
                "false" | "no" | "0" => Some(false),
                _ => None,
            },
            Self::Number(n) => Some(*n != 0.0),
            _ => None,
        }
    }

    /// The value as an `f64`, parsing numeric strings.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            Self::String(s) => s.trim().parse().ok(),
            Self::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    /// The value as an `f32`.
    pub fn as_f32(&self) -> Option<f32> {
        self.as_f64().map(|n| n as f32)
    }

    /// The value as a string slice. Does not stringify numbers — use
    /// [`TagValue::as_f64`] for those, so a typo'd type is an error rather
    /// than a silently coerced value.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// The value as a list.
    pub fn as_list(&self) -> Option<&[TagValue]> {
        match self {
            Self::List(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    /// The value as a nested object.
    pub fn as_map(&self) -> Option<&BTreeMap<String, TagValue>> {
        match self {
            Self::Map(m) => Some(m),
            _ => None,
        }
    }

    /// The value as a [`Vec3`], accepting either `[x, y, z]` or
    /// `{"x": .., "y": .., "z": ..}` since both conventions appear in the wild.
    pub fn as_vec3(&self) -> Option<Vec3> {
        match self {
            Self::List(v) if v.len() == 3 => {
                Some(Vec3::new(v[0].as_f32()?, v[1].as_f32()?, v[2].as_f32()?))
            }
            Self::Map(m) => Some(Vec3::new(
                m.get("x")?.as_f32()?,
                m.get("y")?.as_f32()?,
                m.get("z")?.as_f32()?,
            )),
            _ => None,
        }
    }

    /// Whether this is [`TagValue::Null`].
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The name of this variant, for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::List(_) => "list",
            Self::Map(_) => "map",
        }
    }
}

/// The parameters attached to a tag: a bag of named [`TagValue`]s.
#[derive(Clone, Debug, PartialEq, Reflect, Default)]
pub struct TagParams(BTreeMap<String, TagValue>);

impl TagParams {
    /// An empty parameter set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a parameter.
    pub fn get(&self, key: &str) -> Option<&TagValue> {
        self.0.get(key)
    }

    /// Look up a `bool` parameter.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_bool()
    }

    /// Look up an `f32` parameter.
    pub fn get_f32(&self, key: &str) -> Option<f32> {
        self.get(key)?.as_f32()
    }

    /// Look up a string parameter.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    /// Look up a [`Vec3`] parameter.
    pub fn get_vec3(&self, key: &str) -> Option<Vec3> {
        self.get(key)?.as_vec3()
    }

    /// Whether a parameter is present.
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// Iterate the parameters in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &TagValue)> {
        self.0.iter()
    }

    /// The number of parameters.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no parameters.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Insert a parameter, returning any previous value.
    pub fn insert(&mut self, key: impl Into<String>, value: TagValue) -> Option<TagValue> {
        self.0.insert(key.into(), value)
    }

    /// Remove a parameter.
    pub fn remove(&mut self, key: &str) -> Option<TagValue> {
        self.0.remove(key)
    }
}

impl From<BTreeMap<String, TagValue>> for TagParams {
    fn from(map: BTreeMap<String, TagValue>) -> Self {
        Self(map)
    }
}

impl FromIterator<(String, TagValue)> for TagParams {
    fn from_iter<I: IntoIterator<Item = (String, TagValue)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// Reduce a name to a form that ignores the casing and separator conventions
/// that differ between DCCs and Rust.
///
/// `SpawnPoint`, `spawn_point`, `spawn-point` and `Spawn Point` all collapse to
/// `spawnpoint`, so an artist's naming habits never silently fail to match a
/// tag the game registered.
pub fn normalize_key(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// A case- and separator-insensitive lookup table keyed by [`normalize_key`].
#[derive(Debug, Clone)]
pub struct NormalizedMap<V>(HashMap<String, V>);

// Hand-written so that an empty map does not require `V: Default`.
impl<V> Default for NormalizedMap<V> {
    fn default() -> Self {
        Self(HashMap::default())
    }
}

impl<V> NormalizedMap<V> {
    /// Insert under the normalized form of `key`.
    pub fn insert(&mut self, key: &str, value: V) -> Option<V> {
        self.0.insert(normalize_key(key), value)
    }

    /// Look up by any spelling of `key`.
    pub fn get(&self, key: &str) -> Option<&V> {
        self.0.get(&normalize_key(key))
    }

    /// Whether any spelling of `key` is present.
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(&normalize_key(key))
    }

    /// Iterate normalized keys and values.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.0.iter()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A failure turning tag parameters into a concrete type.
#[derive(Debug, Error)]
#[error("could not read tag parameters as `{type_name}`: {message}")]
pub struct TagDeserializeError {
    /// The type that was being built.
    pub type_name: &'static str,
    /// What went wrong.
    pub message: String,
}

impl TagValue {
    /// Convert to `serde_json`'s value model.
    ///
    /// Used as the bridge to `serde`: rather than hand-writing a `Deserializer`
    /// for [`TagValue`], parameters are handed to any `Deserialize` type
    /// through `serde_json`, which every Rust game developer already
    /// understands the error messages of.
    pub fn to_json(&self) -> JsonValue {
        match self {
            Self::Null => JsonValue::Null,
            Self::Bool(b) => JsonValue::Bool(*b),
            Self::Number(n) => serde_json::Number::from_f64(*n)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
            Self::String(s) => JsonValue::String(s.clone()),
            Self::List(v) => JsonValue::Array(v.iter().map(TagValue::to_json).collect()),
            Self::Map(m) => {
                JsonValue::Object(m.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
            }
        }
    }
}

impl TagParams {
    /// Convert to a JSON object.
    pub fn to_json(&self) -> JsonValue {
        JsonValue::Object(self.0.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
    }

    /// Build a `Deserialize` type from these parameters.
    ///
    /// Missing fields are the caller's problem to handle with `#[serde(default)]`;
    /// an artist who tags a node without filling in every property should get a
    /// sensible default, not a failed level load.
    pub fn deserialize<T: DeserializeOwned>(&self) -> Result<T, TagDeserializeError> {
        serde_json::from_value(self.to_json()).map_err(|e| TagDeserializeError {
            type_name: std::any::type_name::<T>(),
            message: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_accepts_the_spellings_dccs_actually_export() {
        assert_eq!(TagValue::Bool(true).as_bool(), Some(true));
        assert_eq!(TagValue::String("True".into()).as_bool(), Some(true));
        assert_eq!(TagValue::String(" no ".into()).as_bool(), Some(false));
        assert_eq!(TagValue::Number(1.0).as_bool(), Some(true));
        assert_eq!(TagValue::String("maybe".into()).as_bool(), None);
    }

    #[test]
    fn numbers_parse_from_strings_but_strings_do_not_come_from_numbers() {
        assert_eq!(TagValue::String("2.5".into()).as_f32(), Some(2.5));
        // A number is not silently readable as a string: that would turn a
        // type mismatch into a confusing value rather than a clear failure.
        assert_eq!(TagValue::Number(2.5).as_str(), None);
    }

    #[test]
    fn vec3_accepts_both_array_and_object_forms() {
        let list = TagValue::List(vec![
            TagValue::Number(1.0),
            TagValue::Number(2.0),
            TagValue::Number(3.0),
        ]);
        assert_eq!(list.as_vec3(), Some(Vec3::new(1.0, 2.0, 3.0)));

        let map = TagValue::Map(BTreeMap::from([
            ("x".to_string(), TagValue::Number(1.0)),
            ("y".to_string(), TagValue::Number(2.0)),
            ("z".to_string(), TagValue::Number(3.0)),
        ]));
        assert_eq!(map.as_vec3(), Some(Vec3::new(1.0, 2.0, 3.0)));

        let too_short = TagValue::List(vec![TagValue::Number(1.0)]);
        assert_eq!(too_short.as_vec3(), None);
    }

    #[test]
    fn normalization_collapses_dcc_and_rust_naming_conventions() {
        for spelling in ["SpawnPoint", "spawn_point", "spawn-point", "Spawn Point"] {
            assert_eq!(normalize_key(spelling), "spawnpoint", "{spelling}");
        }
        // Distinct tags must stay distinct.
        assert_ne!(normalize_key("spawn_point"), normalize_key("spawn_points"));
    }

    #[test]
    fn params_deserialize_into_a_concrete_type() {
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct Spawn {
            team: String,
            #[serde(default)]
            weight: f32,
        }

        let params = TagParams::from_iter([
            ("team".to_string(), TagValue::String("red".into())),
            ("weight".to_string(), TagValue::Number(2.5)),
        ]);
        assert_eq!(
            params.deserialize::<Spawn>().unwrap(),
            Spawn { team: "red".into(), weight: 2.5 }
        );

        // A field the artist left out falls back to the serde default rather
        // than failing the load.
        let sparse = TagParams::from_iter([("team".to_string(), TagValue::String("blue".into()))]);
        assert_eq!(sparse.deserialize::<Spawn>().unwrap().weight, 0.0);

        // A genuinely wrong type is still an error, naming the target type.
        let wrong = TagParams::from_iter([("team".to_string(), TagValue::Number(1.0))]);
        let err = wrong.deserialize::<Spawn>().unwrap_err();
        assert!(err.type_name.contains("Spawn"), "{err}");
    }
}
