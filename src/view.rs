//! Small file-following descriptors with separate, lossless full model files.
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, write::GzEncoder};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const COMPRESS_THRESHOLD: usize = 8 * 1024 * 1024;

/// Derive the payload path before validating any output or loading an input.
pub fn model_path(descriptor: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_owned());
    }
    let stem = descriptor
        .file_stem()
        .context("Viewer descriptor has no filename")?;
    let mut name = stem.to_os_string();
    name.push(".model.json");
    Ok(descriptor.with_file_name(name))
}

/// The descriptor contains only fixed-shape counts and local filenames.
pub fn descriptor(envelope: &Value, model_path: &Path) -> Result<Value> {
    let engineering = &envelope["engineering"];
    let count = |field: &str| engineering[field].as_u64().unwrap_or(0);
    let items = |path: &str| {
        engineering
            .pointer(path)
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    };
    let metrics = &envelope["mesh"]["metrics"];
    let model_file = model_path
        .file_name()
        .context("Viewer model has no filename")?
        .to_string_lossy();
    let source = envelope["source"]["name"]
        .as_str()
        .context("Viewer model has no source name")?;
    let source: String = source.chars().take(256).collect();
    let document_id = envelope["document_id"]
        .as_str()
        .context("Viewer model has no document ID")?;
    Ok(
        json!({"schema_version":1,"kind":"revit-view-descriptor","document_id":document_id,
        "source":{"name":source},"model_file":model_file,
        "summary":{"elem_table_total":count("elem_table_total"),
            "elem_table_coverage_pct":engineering["elem_table_coverage_pct"].as_f64().unwrap_or(0.0),
            "electrical_circuits":items("/electrical_engineering/circuits"),
            "panel_schedules":engineering["electrical_engineering"]["panel_schedules_count"].as_u64().unwrap_or(0),
            "electrical_equipment":items("/electrical_engineering/panels_and_equipment"),
            "hvac_systems":items("/hvac_engineering/hvac_systems"),
            "mechanical_equipment":items("/hvac_engineering/mechanical_equipment"),
            "sheets":count("sheets_count"),"schedules":count("schedules_count"),"text_notes":count("text_notes_count"),
            "mesh_nodes":metrics["drawable_node_count"].as_u64().unwrap_or(0),
            "drawable_element_ids":metrics["element_ids"].as_array().map_or(0,Vec::len)}}),
    )
}

/// Publish the full payload first so a newly visible descriptor always names it.
/// A new document ID prevents selecting an older or unrelated model file.
pub fn write_files(descriptor_path: &Path, model_path: &Path, envelope: &mut Value) -> Result<()> {
    crate::output::validate(&[], &[descriptor_path, model_path])?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    envelope["document_id"] = json!(format!("{nanos:x}-{}", std::process::id()));
    let descriptor = descriptor(envelope, model_path)?;
    crate::output::write_json(model_path, envelope)?;
    crate::output::write_json(descriptor_path, &descriptor)
}

pub fn mesh_payload(gltf: Value, metrics: Value) -> Result<Value> {
    encode_mesh(gltf, metrics, COMPRESS_THRESHOLD)
}

fn encode_mesh(gltf: Value, metrics: Value, threshold: usize) -> Result<Value> {
    let bytes = serde_json::to_vec(&gltf)?;
    if bytes.len() < threshold {
        return Ok(json!({"gltf":gltf,"metrics":metrics}));
    }
    let mut writer = GzEncoder::new(Vec::new(), Compression::default());
    writer.write_all(&bytes)?;
    let encoded = STANDARD.encode(writer.finish()?);
    Ok(json!({"gltf_gzip_base64":encoded,"metrics":metrics}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn compressed_mesh_roundtrip_keeps_every_json_value_without_raw_duplicate() {
        let gltf = json!({"asset":{"version":"2.0"},"nodes":[{"extras":{"elementId":"9000000001"}}],"buffers":[{"uri":"data:application/octet-stream;base64,AA=="}]});
        let payload = encode_mesh(gltf.clone(), json!({"triangle_count":42}), 0).unwrap();
        assert!(payload.get("gltf").is_none());
        let compressed = STANDARD
            .decode(payload["gltf_gzip_base64"].as_str().unwrap())
            .unwrap();
        let mut decoder = flate2::read::GzDecoder::new(compressed.as_slice());
        let mut bytes = Vec::new();
        decoder.read_to_end(&mut bytes).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), gltf);
        assert_eq!(payload["metrics"]["triangle_count"], 42);
    }
    #[test]
    fn small_mesh_keeps_readable_gltf_contract() {
        let gltf = json!({"asset":{"version":"2.0"}});
        let payload = mesh_payload(gltf.clone(), Value::Null).unwrap();
        assert_eq!(payload["gltf"], gltf);
        assert!(payload.get("gltf_gzip_base64").is_none());
    }
}
