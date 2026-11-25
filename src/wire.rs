//! One lossless ElementId boundary for JSON consumers, including JavaScript.
//! Actual element/definition IDs are decimal strings. Negative built-in enum
//! IDs and sentinel values remain numbers; counts and physical values are never IDs.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

const SCALAR_IDS: &[&str] = &[
    "element_id",
    "owner_id",
    "parameter_id",
    "source_element_id",
    "owner_view_id",
    "panel_id",
    "target_panel_id",
    "schedule_view_id",
    "upstream_circuit_id",
    "supply_from_panel_id",
    "native_space_id",
    "type_id",
    "level_id",
    "family_id",
    "symbol_id",
    "demand_factor_id",
];
const ARRAY_IDS: &[&str] = &[
    "upstream_ids",
    "downstream_ids",
    "connected_device_ids",
    "element_ids",
    "failed_element_ids",
    "inexact_element_ids",
];

fn identifier(value: &mut Value) -> Result<()> {
    if value.is_null() {
        return Ok(());
    }
    let id = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok()))
        .context("An ElementId must be an exact signed 64-bit integer")?;
    *value = if id >= 0 {
        json!(id.to_string())
    } else {
        json!(id)
    };
    Ok(())
}

pub fn encode_element_ids(value: &mut Value) -> Result<()> {
    encode(value, 0)
}

fn encode(value: &mut Value, depth: usize) -> Result<()> {
    ensure!(depth <= 64, "Engineering JSON nesting limit");
    match value {
        Value::Object(object) => {
            let parameter_id_value = object.get("storage_type").is_some_and(|v| v == "ElementId");
            for (key, value) in object {
                if SCALAR_IDS.contains(&key.as_str()) || (key == "value" && parameter_id_value) {
                    identifier(value)?;
                } else if ARRAY_IDS.contains(&key.as_str()) && !value.is_null() {
                    for id in value
                        .as_array_mut()
                        .context("ElementId list must be an array")?
                    {
                        identifier(id)?;
                    }
                } else {
                    encode(value, depth + 1)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                encode(value, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
