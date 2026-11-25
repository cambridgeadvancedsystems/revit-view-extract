//! Saved instance/type parameters enriched with numeric enums, FKSA units and references.
use crate::model::{RevitModel, Table, field_id, text, truthy};
use crate::units::{convert_parameter, round};
use anyhow::{Context, Result, ensure};
use rvt::{RevitFile, research::native_element_parameters};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn read_native(file: &mut RevitFile, model: &RevitModel) -> Result<(Table, Value)> {
    let ids: BTreeSet<_> = model
        .declared_ids
        .iter()
        .filter_map(|id| u64::try_from(*id).ok())
        .collect();
    ensure!(
        file.layout_release()? == 2024,
        "Engineering extraction is verified for Revit 2024 only; pre-2024 ElementId layouts and later releases are unsupported"
    );
    let saved =
        native_element_parameters::read(file, &ids).context("Read saved native parameters")?;
    ensure!(
        file.unread_streams().is_empty(),
        "Native parameter extraction stopped at a resource limit"
    );
    let record_count = saved.elements.len();
    let mut parameters = Table::new();
    for (id, values) in saved.elements {
        let mut values = serde_json::to_value(values)?;
        for param in values
            .as_array_mut()
            .context("Parameter reader returned a non-array")?
        {
            enrich(model, param)?;
        }
        if values.as_array().is_some_and(|values| !values.is_empty()) {
            parameters.insert(i64::try_from(id)?, values);
        }
    }
    let report = measure(
        &parameters,
        &json!({"parameter_records_read":record_count,
        "unresolved_family_slots":saved.unresolved_family_slots,"instance_parameters_without_value":saved.instance_parameters_without_value}),
    );
    Ok((parameters, report))
}

pub fn enrich(model: &RevitModel, param: &mut Value) -> Result<()> {
    let id = field_id(param, "parameter_id").context("Saved parameter has no exact ID")?;
    let storage = param["storage_type"]
        .as_str()
        .context("Saved parameter has no storage type")?
        .to_owned();
    ensure!(
        ["String", "Integer", "Double", "ElementId"].contains(&storage.as_str()),
        "Unknown saved parameter storage type {storage}"
    );
    let fields = param
        .as_object_mut()
        .context("Saved parameter must be an object")?;
    for field in ["converted_value", "unit", "secondary_values", "target_kind"] {
        fields.remove(field);
    }
    if id < 0 {
        param["name"] = model
            .enums
            .parameter_name(id)
            .map(|name| json!(name))
            .unwrap_or(Value::Null);
    } else if !truthy(&param["name"]) {
        param["name"] = model
            .param_defs
            .get(&id)
            .map(|p| p["caption"].clone())
            .unwrap_or(Value::Null);
    }
    if storage == "Double"
        && let Some(converted) = param["value"]
            .as_f64()
            .and_then(|value| convert_parameter(param["spec_type_id"].as_str(), id, value))
    {
        param["converted_value"] = json!(converted.value);
        param["unit"] = json!(converted.unit);
        param["secondary_values"] = converted.secondary;
    }
    if storage == "ElementId" {
        param["target_name"] = Value::Null;
        if let Some((name, kind)) = param["value"]
            .as_i64()
            .and_then(|id| model.resolve_element_id(id))
        {
            param["target_name"] = json!(name);
            param["target_kind"] = json!(kind);
        }
    }
    Ok(())
}

pub fn measure(parameters: &Table, diagnostics: &Value) -> Value {
    let mut total = 0;
    let mut named = 0;
    let mut doubles = 0;
    let mut converted = 0;
    let mut references = 0;
    let mut resolved = 0;
    let mut unresolved_names: BTreeMap<String, usize> = BTreeMap::new();
    let mut units: BTreeMap<String, usize> = BTreeMap::new();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut unresolved_refs: BTreeMap<String, usize> = BTreeMap::new();
    for values in parameters.values().filter_map(Value::as_array) {
        for param in values {
            total += 1;
            let id = field_id(param, "parameter_id").unwrap_or(0);
            if truthy(&param["name"]) {
                named += 1;
            } else {
                *unresolved_names.entry(id.to_string()).or_default() += 1;
            }
            if param["storage_type"] == "Double" {
                doubles += 1;
                if param["converted_value"]
                    .as_f64()
                    .is_some_and(f64::is_finite)
                    && param["unit"].is_string()
                {
                    converted += 1;
                    *units.entry(text(param, "unit")).or_default() += 1;
                }
            }
            if param["storage_type"] == "ElementId"
                && let Some(id) = param["value"].as_i64().filter(|id| *id != -1)
            {
                references += 1;
                if let Some(kind) = param["target_kind"].as_str() {
                    resolved += 1;
                    *kinds.entry(kind.to_owned()).or_default() += 1;
                } else {
                    *unresolved_refs.entry(id.to_string()).or_default() += 1;
                }
            }
        }
    }
    let pct = |a: usize, b: usize| {
        if b > 0 {
            round(a as f64 / b as f64 * 100.0, 2)
        } else {
            0.0
        }
    };
    let mut report = json!({"elements_with_parameters":parameters.values().filter(|v|v.as_array().is_some_and(|v|!v.is_empty())).count(),"total_parameters":total,
        "named_parameters":named,"named_pct":pct(named,total),"unresolved_parameter_ids":unresolved_names,
        "double_parameters":doubles,"converted_parameters":converted,"converted_pct":pct(converted,doubles),"converted_by_unit":units,
        "nonnull_elementid_parameters":references,"resolved_elementid_parameters":resolved,"resolved_pct":pct(resolved,references),
        "resolved_by_target_kind":kinds,"unresolved_element_ids":unresolved_refs});
    for field in [
        "parameter_records_read",
        "unresolved_family_slots",
        "instance_parameters_without_value",
    ] {
        if let Some(value) = diagnostics.get(field) {
            report[field] = value.clone();
        }
    }
    report
}
