//! Turning raw glTF `extras` JSON and node names into [`StageTags`].
//!
//! Everything here is pure: no Bevy world access, no registry. That makes the
//! whole authoring surface unit-testable without booting an app, which matters
//! because this is where artist mistakes surface.

use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use thiserror::Error;

use super::component::{StageTag, StageTags, TagSource};
use super::value::{TagParams, TagValue};

/// The default `extras` property that names a node's tags.
pub const DEFAULT_DISCRIMINATOR: &str = "stage_tag";

/// A failure while reading a node's metadata.
#[derive(Debug, Error, PartialEq)]
pub enum TagParseError {
    /// The `extras` blob was not valid JSON.
    #[error("extras is not valid JSON: {0}")]
    InvalidJson(String),
    /// The `extras` blob parsed but was not a JSON object.
    #[error("extras must be a JSON object, found {0}")]
    NotAnObject(&'static str),
    /// The discriminator property held something other than a tag name or a
    /// list of tag names.
    #[error("`{discriminator}` must be a string or list of strings, found {found}")]
    BadDiscriminator {
        /// The discriminator property name in use.
        discriminator: String,
        /// The type actually found.
        found: &'static str,
    },
}

/// Convert parsed JSON into the plugin's value model.
pub fn json_to_tag_value(value: &JsonValue) -> TagValue {
    match value {
        JsonValue::Null => TagValue::Null,
        JsonValue::Bool(b) => TagValue::Bool(*b),
        // Any JSON number fits in f64 for authoring purposes; the only loss is
        // beyond 2^53, which no artist-authored property will reach.
        JsonValue::Number(n) => TagValue::Number(n.as_f64().unwrap_or(0.0)),
        JsonValue::String(s) => TagValue::String(s.clone()),
        JsonValue::Array(a) => TagValue::List(a.iter().map(json_to_tag_value).collect()),
        JsonValue::Object(o) => TagValue::Map(
            o.iter()
                .map(|(k, v)| (k.clone(), json_to_tag_value(v)))
                .collect::<BTreeMap<_, _>>(),
        ),
    }
}

fn json_type_name(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "bool",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "list",
        JsonValue::Object(_) => "object",
    }
}

/// Parse a node's `extras` blob.
///
/// Returns the full property set plus any tags named by the discriminator. A
/// tag's parameters are its *sibling* properties, because Blender's custom
/// property UI is flat — asking artists to type nested JSON would not survive
/// contact with a real art team.
pub fn parse_extras(
    text: &str,
    discriminator: &str,
) -> Result<(TagParams, Vec<StageTag>), TagParseError> {
    let json: JsonValue =
        serde_json::from_str(text).map_err(|e| TagParseError::InvalidJson(e.to_string()))?;

    let JsonValue::Object(object) = &json else {
        return Err(TagParseError::NotAnObject(json_type_name(&json)));
    };

    let mut extras = TagParams::new();
    for (key, value) in object {
        extras.insert(key.clone(), json_to_tag_value(value));
    }

    let Some(raw) = object.get(discriminator) else {
        return Ok((extras, Vec::new()));
    };

    let names: Vec<String> = match raw {
        JsonValue::String(s) => vec![s.clone()],
        JsonValue::Array(items) => {
            let mut names = Vec::with_capacity(items.len());
            for item in items {
                let JsonValue::String(s) = item else {
                    return Err(TagParseError::BadDiscriminator {
                        discriminator: discriminator.to_string(),
                        found: json_type_name(item),
                    });
                };
                names.push(s.clone());
            }
            names
        }
        other => {
            return Err(TagParseError::BadDiscriminator {
                discriminator: discriminator.to_string(),
                found: json_type_name(other),
            })
        }
    };

    // Sibling properties become the parameters, shared by every tag the
    // discriminator names.
    let mut params = TagParams::new();
    for (key, value) in object {
        if key != discriminator {
            params.insert(key.clone(), json_to_tag_value(value));
        }
    }

    let tags = names
        .into_iter()
        .map(|key| StageTag {
            key,
            params: params.clone(),
            source: TagSource::Extras,
        })
        .collect();

    Ok((extras, tags))
}

/// Split a glTF node name into its base name and any inline parameters.
///
/// Handles the two things that make raw node names unusable as-is:
/// Blender's `.001` suffix on duplicated objects, and an optional
/// `Name[key=value,flag]` block for artists who want parameters without
/// touching custom properties.
pub fn parse_node_name(raw: &str) -> (String, TagParams) {
    let without_suffix = strip_duplicate_suffix(raw);

    let Some(open) = without_suffix.find('[') else {
        return (without_suffix.trim().to_string(), TagParams::new());
    };
    let Some(close) = without_suffix.rfind(']') else {
        return (without_suffix.trim().to_string(), TagParams::new());
    };
    if close < open {
        return (without_suffix.trim().to_string(), TagParams::new());
    }

    let base = without_suffix[..open].trim().to_string();
    let mut params = TagParams::new();
    for entry in without_suffix[open + 1..close].split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        match entry.split_once('=') {
            Some((key, value)) => {
                params.insert(key.trim().to_string(), parse_scalar(value.trim()));
            }
            // A bare word is a flag: `Door[locked]` reads better than
            // `Door[locked=true]` and means the same thing.
            None => {
                params.insert(entry.to_string(), TagValue::Bool(true));
            }
        }
    }
    (base, params)
}

/// Strip a trailing `.001`-style suffix, which Blender appends to duplicates.
fn strip_duplicate_suffix(raw: &str) -> &str {
    let trimmed = raw.trim_end();
    let Some((head, tail)) = trimmed.rsplit_once('.') else {
        return trimmed;
    };
    if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) && !head.is_empty() {
        head
    } else {
        trimmed
    }
}

/// Interpret an inline parameter value, preferring the most specific type.
fn parse_scalar(raw: &str) -> TagValue {
    match raw.to_ascii_lowercase().as_str() {
        "true" => return TagValue::Bool(true),
        "false" => return TagValue::Bool(false),
        _ => {}
    }
    if let Ok(n) = raw.parse::<f64>() {
        return TagValue::Number(n);
    }
    // Allow quoting to force a string, so `label="12"` stays text.
    let unquoted = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(raw);
    TagValue::String(unquoted.to_string())
}

/// Build the component baked into the loaded asset for one node.
///
/// A malformed `extras` blob is reported rather than propagated: one bad
/// property on one prop should not fail the whole level load.
pub fn build_stage_tags(
    name: &str,
    extras_json: Option<&str>,
    discriminator: &str,
) -> (StageTags, Option<TagParseError>) {
    let (base_name, name_params) = parse_node_name(name);
    let mut tags = StageTags {
        base_name,
        name_params,
        ..Default::default()
    };

    let Some(text) = extras_json else {
        return (tags, None);
    };

    match parse_extras(text, discriminator) {
        Ok((extras, parsed)) => {
            tags.extras = extras;
            tags.tags = parsed;
            (tags, None)
        }
        Err(error) => (tags, Some(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blender_duplicate_suffixes_are_stripped() {
        assert_eq!(strip_duplicate_suffix("SpawnPoint.001"), "SpawnPoint");
        assert_eq!(strip_duplicate_suffix("SpawnPoint"), "SpawnPoint");
        // A dot followed by non-digits is part of the name, not a suffix.
        assert_eq!(strip_duplicate_suffix("Props.Crate"), "Props.Crate");
        // A leading dot is not a suffix marker either.
        assert_eq!(strip_duplicate_suffix(".001"), ".001");
    }

    #[test]
    fn inline_name_params_parse_with_types() {
        let (base, params) = parse_node_name("SpawnPoint[team=red,weight=2.5,primary].001");
        assert_eq!(base, "SpawnPoint");
        assert_eq!(params.get_str("team"), Some("red"));
        assert_eq!(params.get_f32("weight"), Some(2.5));
        // A bare word is a flag.
        assert_eq!(params.get_bool("primary"), Some(true));
    }

    #[test]
    fn quoting_forces_a_numeric_looking_value_to_stay_a_string() {
        let (_, params) = parse_node_name(r#"Sign[label="12"]"#);
        assert_eq!(params.get_str("label"), Some("12"));
    }

    #[test]
    fn a_plain_name_yields_no_params() {
        let (base, params) = parse_node_name("Ground");
        assert_eq!(base, "Ground");
        assert!(params.is_empty());
    }

    #[test]
    fn discriminator_promotes_siblings_to_params() {
        let json = r#"{"stage_tag": "spawn_point", "team": "blue", "weight": 3}"#;
        let (extras, tags) = parse_extras(json, DEFAULT_DISCRIMINATOR).unwrap();

        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].key, "spawn_point");
        assert_eq!(tags[0].source, TagSource::Extras);
        assert_eq!(tags[0].params.get_str("team"), Some("blue"));
        assert_eq!(tags[0].params.get_f32("weight"), Some(3.0));
        // The discriminator itself is not a parameter of its own tag.
        assert!(!tags[0].params.contains("stage_tag"));
        // ...but the raw extras keep everything, including the discriminator.
        assert!(extras.contains("stage_tag"));
    }

    #[test]
    fn one_node_can_carry_several_tags() {
        let json = r#"{"stage_tag": ["anchor", "portal"], "id": "north"}"#;
        let (_, tags) = parse_extras(json, DEFAULT_DISCRIMINATOR).unwrap();

        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].key, "anchor");
        assert_eq!(tags[1].key, "portal");
        // Shared siblings reach both tags.
        assert_eq!(tags[1].params.get_str("id"), Some("north"));
    }

    #[test]
    fn extras_without_a_discriminator_still_parse() {
        let json = r#"{"SpawnPoint": "(team: \"red\")"}"#;
        let (extras, tags) = parse_extras(json, DEFAULT_DISCRIMINATOR).unwrap();

        // No tags, but the property is preserved for the reflection channel to
        // resolve later against the type registry.
        assert!(tags.is_empty());
        assert_eq!(extras.get_str("SpawnPoint"), Some(r#"(team: "red")"#));
    }

    #[test]
    fn malformed_extras_are_reported_not_panicked() {
        assert!(matches!(
            parse_extras("{not json", DEFAULT_DISCRIMINATOR),
            Err(TagParseError::InvalidJson(_))
        ));
        assert_eq!(
            parse_extras("[1,2,3]", DEFAULT_DISCRIMINATOR),
            Err(TagParseError::NotAnObject("list"))
        );
        assert_eq!(
            parse_extras(r#"{"stage_tag": 5}"#, DEFAULT_DISCRIMINATOR),
            Err(TagParseError::BadDiscriminator {
                discriminator: DEFAULT_DISCRIMINATOR.to_string(),
                found: "number",
            })
        );
    }

    #[test]
    fn a_bad_extras_blob_still_yields_usable_name_data() {
        // The level should survive one artist's typo: the node keeps its
        // identity and the error is surfaced separately.
        let (tags, error) = build_stage_tags("Door.002", Some("{oops"), DEFAULT_DISCRIMINATOR);
        assert_eq!(tags.base_name, "Door");
        assert!(error.is_some());
    }

    #[test]
    fn nested_objects_and_lists_survive_the_round_trip() {
        let json = r#"{"stage_tag": "sector", "bounds": {"x": 1, "y": 2, "z": 3}, "tags": ["a", "b"]}"#;
        let (_, tags) = parse_extras(json, DEFAULT_DISCRIMINATOR).unwrap();
        assert_eq!(
            tags[0].params.get_vec3("bounds"),
            Some(bevy::prelude::Vec3::new(1.0, 2.0, 3.0))
        );
        assert_eq!(tags[0].params.get("tags").unwrap().as_list().unwrap().len(), 2);
    }
}
