//! Patch inputs retain legacy JSON parsing/error behavior, with an explicit schema for clients.
use super::{Pod, Settings};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::Deserialize;
use serde_json::Value;
use std::borrow::Cow;

#[derive(Deserialize)]
#[serde(transparent)]
pub struct PodPatch(pub Value);

impl JsonSchema for PodPatch {
    fn schema_name() -> Cow<'static, str> {
        "PodPatch".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = Pod::json_schema(generator);
        let object = schema.as_object_mut().unwrap();
        object.remove("required");
        object.insert("additionalProperties".into(), Value::Bool(false));
        let properties = object
            .get_mut("properties")
            .unwrap()
            .as_object_mut()
            .unwrap();
        // Identity is allocated by the backend; the obsolete bar material cannot be patched.
        properties.remove("id");
        properties.remove("material");
        for property in properties.values_mut() {
            if matches!(property["type"].as_str(), Some("number" | "integer")) {
                *property = serde_json::json!({"anyOf": [property.clone(), {"type": "string"}]});
            }
        }
        schema
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
pub struct SettingsPatch(pub Value);

impl JsonSchema for SettingsPatch {
    fn schema_name() -> Cow<'static, str> {
        "SettingsPatch".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = Settings::json_schema(generator);
        let object = schema.as_object_mut().unwrap();
        object.remove("required");
        object.insert("additionalProperties".into(), Value::Bool(false));
        let properties = object
            .get_mut("properties")
            .unwrap()
            .as_object_mut()
            .unwrap();
        // These historical keys are accepted and deliberately ignored by merge_persist.
        for ignored in ["pods", "version", "dataDir"] {
            properties.insert(ignored.into(), Value::Bool(true));
        }
        schema
    }
}
