//! Real geometry from the pinned Apache-2.0 rvt-rs reader.
//!
//! Saved display meshes and measured analytic profiles are retained. Bodies
//! identified by upstream as bounding-box stand-ins are omitted. The glTF
//! frame is metres, Y up; every drawable node carries its Revit ElementId.

#[path = "mesh/document.rs"]
mod document;
#[path = "mesh/geometry.rs"]
mod geometry;
#[path = "mesh/rooms.rs"]
mod rooms;
#[cfg(test)]
#[path = "mesh/tests.rs"]
mod tests;

pub use rooms::architectural_rooms;

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use rvt::RevitFile;
use rvt::ifc::entities::{IfcEntity, PropertySet, PropertyValue};
use rvt::ifc::{
    ExportDiagnostics, ExportOptions, IfcModel, SavedMeshChoice, gltf::build_gltf, run_export,
};
use serde::Serialize;
use serde::ser::SerializeMap;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

pub const UPSTREAM_COMMIT: &str = "eb268fdd22ced8e43bf8b2236b582f182880a2d8";

#[derive(Debug, Clone, Serialize)]
pub struct MeshReport {
    pub exporter: &'static str,
    pub upstream_commit: &'static str,
    pub coordinate_system: &'static str,
    pub triangle_count: usize,
    pub vertex_count: usize,
    pub primitive_count: usize,
    pub drawable_node_count: usize,
    pub element_ids: Vec<String>,
    pub bounds_meters: document::Bounds,
    pub body_sources: BTreeMap<String, usize>,
    pub omitted_proxy_element_ids: Vec<String>,
    pub omitted_proxy_sources: BTreeMap<String, usize>,
    pub omitted_unidentified_nodes: usize,
    pub omitted_geometry_reasons: BTreeMap<String, Vec<String>>,
    pub diagnostics: Value,
    pub notes: Vec<String>,
}

/// An embedded-buffer glTF document and the measurements of its actual mesh.
#[derive(Debug, Serialize)]
pub struct MeshOutput {
    pub gltf: Value,
    pub metrics: MeshReport,
}

/// Extract geometry without writing an intermediate file, for a view envelope.
pub fn build(input: &Path) -> Result<MeshOutput> {
    let mut file = RevitFile::open(input).context("open mesh source")?;
    let options = ExportOptions::default().with_saved_meshes(SavedMeshChoice::All);
    let run = run_export(&mut file, &options).context("recover Revit geometry with rvt-rs")?;
    require_complete_read(&run.diagnostics)?;
    let diagnostics = serde_json::to_value(&run.diagnostics)?;
    let notes = run.notes.iter().map(ToString::to_string).collect();
    from_model(run.model, diagnostics, notes)
}

fn from_model(mut model: IfcModel, diagnostics: Value, notes: Vec<String>) -> Result<MeshOutput> {
    let mut omitted_proxy_element_ids = Vec::new();
    let mut omitted_proxy_sources = BTreeMap::new();
    let mut omitted_unidentified_nodes = 0;
    let mut omitted_geometry_reasons: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut identities = BTreeMap::new();
    for (entity_index, entity) in model.entities.iter_mut().enumerate() {
        let geometry_issue = geometry::issue(entity, &model.representation_maps);
        let IfcEntity::BuildingElement {
            type_guid,
            property_set,
            ifc_type,
            extrusion,
            solid_shape,
            representation_map_index,
            ..
        } = entity
        else {
            continue;
        };
        if extrusion.is_none() && solid_shape.is_none() && representation_map_index.is_none() {
            continue;
        }
        let id = type_guid
            .as_deref()
            .and_then(|id| id.parse::<i64>().ok())
            .filter(|id| *id > 0);
        let source = text_property(property_set.as_ref(), "BodySource").unwrap_or("analytic");
        if let Some(id) = id {
            match (is_proxy(ifc_type, property_set.as_ref()), geometry_issue) {
                (false, None) => {
                    identities.insert(entity_index, (id.to_string(), source.to_owned()));
                    continue;
                }
                (false, Some(issue)) => {
                    omitted_geometry_reasons
                        .entry(issue.into())
                        .or_default()
                        .push(id.to_string());
                }
                (true, _) => {
                    omitted_proxy_element_ids.push(id.to_string());
                    *omitted_proxy_sources.entry(source.to_owned()).or_insert(0) += 1;
                }
            }
        } else {
            omitted_unidentified_nodes += 1;
        }
        // Preserve entity indices used by relationships, but prevent the
        // exporter from tessellating or emitting a stand-in body at all.
        *extrusion = None;
        *solid_shape = None;
        *representation_map_index = None;
    }
    let (document, binary) = build_gltf(&model);
    let measured = document::measure(&document, &binary)?;
    ensure!(
        measured.drawable_node_count > 0,
        "no supported non-proxy geometry recovered"
    );
    let mut gltf = serde_json::to_value(&document)?;
    let mut body_sources = BTreeMap::new();
    let mut element_ids = Vec::new();
    let mut drawn_entities = BTreeSet::new();
    for (index, node) in document.nodes.iter().enumerate() {
        if node.mesh.is_none() {
            continue;
        }
        let entity_index = node
            .extras
            .as_ref()
            .context("mesh node has no entity identity")?
            .entity_index;
        let (id, source) = identities
            .get(&entity_index)
            .context("mesh node has no Revit ElementId")?;
        drawn_entities.insert(entity_index);
        gltf["nodes"][index]["extras"]["elementId"] = json!(id);
        gltf["nodes"][index]["extras"]["bodySource"] = json!(source);
        *body_sources.entry(source.clone()).or_insert(0) += 1;
        element_ids.push(id.clone());
    }
    for (index, (id, _)) in identities {
        if !drawn_entities.contains(&index) {
            omitted_geometry_reasons
                .entry("no_drawable_tessellation".into())
                .or_default()
                .push(id);
        }
    }
    for ids in omitted_geometry_reasons.values_mut() {
        ids.sort();
        ids.dedup();
    }
    gltf["buffers"][0]["uri"] = json!(format!(
        "data:application/octet-stream;base64,{}",
        STANDARD.encode(&binary)
    ));
    element_ids.sort();
    element_ids.dedup();
    omitted_proxy_element_ids.sort();
    omitted_proxy_element_ids.dedup();
    let metrics = MeshReport {
        exporter: "rvt-rs 0.25.0 / Apache-2.0",
        upstream_commit: UPSTREAM_COMMIT,
        coordinate_system: "metres, Y up; root converts Revit feet/Z up",
        triangle_count: measured.triangle_count,
        vertex_count: measured.vertex_count,
        primitive_count: measured.primitive_count,
        drawable_node_count: measured.drawable_node_count,
        element_ids,
        bounds_meters: measured.bounds,
        body_sources,
        omitted_proxy_element_ids,
        omitted_proxy_sources,
        omitted_unidentified_nodes,
        omitted_geometry_reasons,
        diagnostics,
        notes,
    };
    Ok(MeshOutput { gltf, metrics })
}

/// Write a self-contained `.gltf` or `.glb`, keeping element picking metadata.
pub fn export(input: &Path, output: &Path) -> Result<MeshReport> {
    crate::output::validate(&[input], &[output])?;
    let mesh = build(input)?;
    mesh.write(output)?;
    Ok(mesh.metrics)
}

impl MeshOutput {
    pub fn write(&self, output: &Path) -> Result<()> {
        let extension = output
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("");
        let is_glb = extension.eq_ignore_ascii_case("glb");
        ensure!(
            is_glb || extension.eq_ignore_ascii_case("gltf"),
            "mesh output must end in .gltf or .glb"
        );
        crate::output::write_atomic(output, |writer| {
            if is_glb {
                self.write_glb(writer)
            } else {
                serde_json::to_writer(writer, &self.gltf).context("serialize glTF document")
            }
        })
    }

    fn write_glb(&self, writer: &mut (impl Write + ?Sized)) -> Result<()> {
        let buffers = self.gltf["buffers"]
            .as_array()
            .context("mesh buffer descriptors")?;
        ensure!(buffers.len() == 1, "GLB export requires exactly one buffer");
        let Some(encoded) = self.gltf["buffers"][0]["uri"]
            .as_str()
            .and_then(|uri| uri.strip_prefix("data:application/octet-stream;base64,"))
        else {
            bail!("mesh buffer is not an embedded base64 buffer");
        };
        let binary = STANDARD
            .decode(encoded)
            .context("decode embedded mesh buffer")?;
        ensure!(
            buffers[0]["byteLength"].as_u64() == Some(binary.len() as u64),
            "embedded mesh buffer length mismatch"
        );
        // Upstream's typed NodeExtras cannot carry elementId. Preserve our
        // complete JSON while writing the standard GLB container, without
        // cloning the potentially hundreds-of-megabytes data URI.
        let json = serde_json::to_vec(&GlbJson(self.gltf.as_object().context("glTF document")?))?;
        let json_padding = (4 - json.len() % 4) % 4;
        let bin_padding = (4 - binary.len() % 4) % 4;
        let json_length = u32::try_from(json.len() + json_padding)?;
        let bin_length = u32::try_from(binary.len() + bin_padding)?;
        let length = 28u32
            .checked_add(json_length)
            .and_then(|n| n.checked_add(bin_length))
            .context("GLB exceeds 4 GiB")?;
        for word in [0x4654_6c67u32, 2, length, json_length, 0x4e4f_534a] {
            writer.write_all(&word.to_le_bytes())?;
        }
        writer.write_all(&json)?;
        writer.write_all(&[b' '; 3][..json_padding])?;
        writer.write_all(&bin_length.to_le_bytes())?;
        writer.write_all(&0x004e_4942u32.to_le_bytes())?;
        writer.write_all(&binary)?;
        writer.write_all(&[0; 3][..bin_padding])?;
        Ok(())
    }
}

fn require_complete_read(diagnostics: &ExportDiagnostics) -> Result<()> {
    ensure!(
        !diagnostics.is_partial(),
        "geometry reader stopped at a resource limit; refusing an incomplete model"
    );
    ensure!(
        diagnostics.work_budget_stops().is_none(),
        "geometry reader stopped at a work budget; refusing an incomplete model"
    );
    Ok(())
}

struct GlbJson<'a>(&'a serde_json::Map<String, Value>);

impl Serialize for GlbJson<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut document = serializer.serialize_map(Some(self.0.len()))?;
        for (name, value) in self.0 {
            if name == "buffers" {
                document.serialize_entry(name, &[json!({"byteLength": value[0]["byteLength"]})])?;
            } else {
                document.serialize_entry(name, value)?;
            }
        }
        document.end()
    }
}

fn text_property<'a>(set: Option<&'a PropertySet>, name: &str) -> Option<&'a str> {
    set?.properties
        .iter()
        .find_map(|property| match &property.value {
            PropertyValue::Text(value) if property.name == name => Some(value.as_str()),
            _ => None,
        })
}

/// These four pinned upstream BodySource values are documented stand-ins in
/// ifc/saved_meshes.rs::is_replaceable_box. A resolved room outline refines
/// its box; a roof's level block remains a stand-in even with a resolved plan.
fn is_proxy(ifc_type: &str, set: Option<&PropertySet>) -> bool {
    if ifc_type == "IFCSPACE" {
        return text_property(set, "SpaceOutlineSource")
            .is_none_or(|source| source == "record_box");
    }
    let boxed = matches!(
        text_property(set, "BodySource"),
        Some(
            "partition_element_record_bbox"
                | "partition_beam_box_along_axis"
                | "partition_family_instance_turned_box"
                | "partition_family_instance_oriented_box"
        )
    );
    let boolean = |name: &str| {
        set.is_some_and(|set| {
            set.properties.iter().any(|property| {
                property.name == name && matches!(property.value, PropertyValue::Boolean(true))
            })
        })
    };
    boxed && (!boolean("ProfileResolved") || ifc_type == "IFCROOF" || boolean("RedirectedRoof"))
}
