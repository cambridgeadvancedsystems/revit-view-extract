use super::*;
use rvt::ifc::entities::{Extrusion, Property, SolidShape};
use std::io::Cursor;

fn test_directory(label: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let directory = std::env::current_dir()
        .unwrap()
        .join("runs/review-release/mesh-tests")
        .join(format!(
            "{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn element(id: &str, source: &str, resolved: bool) -> IfcEntity {
    // The serde defaults provide unrelated optional IFC fields; this is a
    // real upstream BuildingElement fed through the production exporter.
    let mut entity: IfcEntity = serde_json::from_value(json!({
        "kind": "BuildingElement",
        "ifc_type": "IFCELECTRICAPPLIANCE",
        "name": "Measured body",
        "type_guid": id,
        "location_feet": [10.0, 20.0, 30.0],
        "rotation_radians": 0.0
    }))
    .unwrap();
    if let IfcEntity::BuildingElement {
        extrusion,
        property_set,
        ..
    } = &mut entity
    {
        *extrusion = Some(Extrusion::rectangle(2.0, 4.0, 6.0));
        *property_set = Some(PropertySet {
            name: "RvtElementRecord".into(),
            properties: vec![
                Property {
                    name: "BodySource".into(),
                    value: PropertyValue::Text(source.into()),
                },
                Property {
                    name: "ProfileResolved".into(),
                    value: PropertyValue::Boolean(resolved),
                },
            ],
        });
    }
    entity
}

fn fixture_model() -> IfcModel {
    let mut actual = element("9007199254740993", "saved_graphics_mesh", false);
    if let IfcEntity::BuildingElement { solid_shape, .. } = &mut actual {
        *solid_shape = Some(SolidShape::TriangulatedFaceSet {
            vertices_feet: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            triangles: vec![rvt::ifc::entities::BrepTriangle(0, 1, 2)],
        });
    }
    IfcModel {
        entities: vec![
            actual,
            element("99", "partition_element_record_bbox", false),
        ],
        ..Default::default()
    }
}

fn fixture() -> MeshOutput {
    from_model(fixture_model(), Value::Null, Vec::new()).unwrap()
}

#[test]
fn real_triangles_keep_element_ids_and_world_bounds_while_proxy_boxes_are_removed() {
    let mesh = fixture();
    // rvt-rs draws saved open display surfaces on both sides.
    assert_eq!(mesh.metrics.triangle_count, 2);
    assert_eq!(mesh.metrics.element_ids, ["9007199254740993"]);
    assert_eq!(mesh.metrics.omitted_proxy_element_ids, ["99"]);
    assert_eq!(mesh.gltf["meshes"].as_array().unwrap().len(), 1);
    let drawable = mesh.gltf["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node.get("mesh").is_some())
        .unwrap();
    assert_eq!(drawable["extras"]["elementId"], "9007199254740993");
    let scale = f64::from(0.3048_f32);
    let expected = [10.0 * scale, 30.0 * scale, -21.0 * scale];
    for (actual, expected) in mesh.metrics.bounds_meters.min.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-6);
    }
    assert!(
        mesh.gltf["buffers"][0]["uri"]
            .as_str()
            .unwrap()
            .starts_with("data:application/octet-stream;base64,")
    );
}

#[test]
fn glb_preserves_picking_metadata_and_embeds_exact_binary_length() {
    let mesh = fixture();
    let mut bytes = Cursor::new(Vec::new());
    mesh.write_glb(&mut bytes).unwrap();
    let bytes = bytes.into_inner();
    let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    assert_eq!(&bytes[..4], b"glTF");
    assert_eq!(word(4), 2);
    assert_eq!(word(8), bytes.len());
    let json_length = word(12);
    let document: Value = serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
    assert!(document["buffers"][0].get("uri").is_none());
    assert_eq!(
        document["nodes"][0]["extras"]["elementId"],
        "9007199254740993"
    );
    assert_eq!(word(24 + json_length), 0x004e_4942);
    assert!(
        word(20 + json_length) >= document["buffers"][0]["byteLength"].as_u64().unwrap() as usize
    );
}

#[test]
fn all_documented_box_standins_are_omitted_but_resolved_profiles_are_kept() {
    for source in [
        "partition_element_record_bbox",
        "partition_beam_box_along_axis",
        "partition_family_instance_turned_box",
        "partition_family_instance_oriented_box",
    ] {
        let model = IfcModel {
            entities: vec![
                element("1", "native_profile", true),
                element("2", source, false),
            ],
            ..Default::default()
        };
        let mesh = from_model(model, Value::Null, Vec::new()).unwrap();
        assert_eq!(mesh.metrics.element_ids, ["1"]);
        assert_eq!(mesh.metrics.omitted_proxy_element_ids, ["2"]);
    }
}

#[test]
fn corrupt_triangle_indices_fail_before_emission() {
    let mut model = IfcModel {
        entities: vec![element("1", "native_profile", true)],
        ..Default::default()
    };
    let (document, mut binary) = build_gltf(&model);
    let mesh = &document.meshes[0];
    let accessor = &document.accessors[mesh.primitives[0].indices.unwrap()];
    let view = &document.buffer_views[accessor.buffer_view];
    let offset = view.byte_offset + accessor.byte_offset;
    binary[offset..offset + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(
        document::measure(&document, &binary)
            .unwrap_err()
            .to_string()
            .contains("triangle index")
    );
    model.entities.clear();
    assert!(from_model(model, Value::Null, Vec::new()).is_err());
}

#[test]
fn malformed_revit_input_never_creates_mesh_output() {
    let directory = test_directory("malformed");
    let input = directory.join("malformed.rvt");
    let output = directory.join("output.gltf");
    std::fs::write(&input, b"not a compound Revit file").unwrap();
    assert!(export(&input, &output).is_err());
    assert!(!output.exists());
    std::fs::remove_file(input).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn indexed_bounds_exclude_unused_saved_vertices() {
    let mut model = fixture_model();
    if let IfcEntity::BuildingElement {
        solid_shape: Some(SolidShape::TriangulatedFaceSet { vertices_feet, .. }),
        ..
    } = &mut model.entities[0]
    {
        vertices_feet.push([999.0, 999.0, 999.0]);
    }
    let mesh = from_model(model, Value::Null, Vec::new()).unwrap();
    assert_eq!(mesh.metrics.vertex_count, 4);
    assert_eq!(mesh.metrics.triangle_count, 2);
    assert!((mesh.metrics.bounds_meters.max[0] - 11.0 * f64::from(0.3048_f32)).abs() < 1e-6);
    assert!((mesh.metrics.bounds_meters.max[1] - 30.0 * f64::from(0.3048_f32)).abs() < 1e-6);
}

#[test]
fn invalid_buffer_views_alignment_and_projective_matrices_are_rejected() {
    let (document, binary) = build_gltf(&fixture_model());
    let position = document.meshes[0].primitives[0].attributes["POSITION"];
    let view_index = document.accessors[position].buffer_view;
    let mut invalid = document.clone();
    invalid.buffer_views[view_index].byte_length = usize::MAX;
    assert!(document::measure(&invalid, &binary).is_err());
    let mut invalid = document.clone();
    invalid.accessors[position].byte_offset = 1;
    assert!(document::measure(&invalid, &binary).is_err());
    let mut invalid = document.clone();
    invalid.accessors[position].count = usize::MAX;
    assert!(document::measure(&invalid, &binary).is_err());
    let mut invalid = document.clone();
    invalid.nodes[0].matrix.as_mut().unwrap()[3] = 0.5;
    assert!(document::measure(&invalid, &binary).is_err());
    let mut invalid = document.clone();
    invalid.nodes[0].children.push(0);
    assert!(document::measure(&invalid, &binary).is_err());
}

#[test]
fn nested_affine_transforms_respect_rotated_placement_and_feet_to_y_up_metres() {
    let mut model = fixture_model();
    if let IfcEntity::BuildingElement {
        rotation_radians, ..
    } = &mut model.entities[0]
    {
        *rotation_radians = Some(std::f64::consts::FRAC_PI_2);
    }
    let (mut document, binary) = build_gltf(&model);
    // Remove the fixture proxy for this direct upstream-document measurement.
    document.nodes[1].mesh = None;
    let scene_roots = document.scenes[0].nodes.clone();
    let mut matrix = [0.0; 16];
    for index in [0, 5, 10, 15] {
        matrix[index] = 1.0;
    }
    matrix[12] = 5.0;
    let parent = document.nodes.len();
    document.nodes.push(rvt::ifc::gltf::Node {
        matrix: Some(matrix),
        children: scene_roots,
        ..Default::default()
    });
    document.scenes[0].nodes = vec![parent];
    let measured = document::measure(&document, &binary).unwrap();
    let scale = f64::from(0.3048_f32);
    let min = [9.0 * scale + 5.0, 30.0 * scale, -21.0 * scale];
    let max = [10.0 * scale + 5.0, 30.0 * scale, -20.0 * scale];
    for (actual, expected) in measured
        .bounds
        .min
        .into_iter()
        .zip(min)
        .chain(measured.bounds.max.into_iter().zip(max))
    {
        assert!((actual - expected).abs() < 1e-6);
    }
}

#[test]
fn resolved_rectangular_profiles_are_kept_while_roof_and_space_standins_are_removed() {
    let mut slab = element("1", "partition_element_record_bbox", true);
    let mut roof = element("2", "partition_element_record_bbox", true);
    let mut space = element("3", "partition_element_record_bbox", true);
    let mut room = element("4", "partition_element_record_bbox", true);
    let mut redirected_roof = element("5", "partition_element_record_bbox", true);
    for (entity, kind, property) in [
        (&mut slab, "IFCSLAB", None),
        (&mut roof, "IFCROOF", None),
        (
            &mut space,
            "IFCSPACE",
            Some((
                "SpaceOutlineSource",
                PropertyValue::Text("record_box".into()),
            )),
        ),
        (
            &mut room,
            "IFCSPACE",
            Some((
                "SpaceOutlineSource",
                PropertyValue::Text("stored_solid".into()),
            )),
        ),
        (
            &mut redirected_roof,
            "IFCSLAB",
            Some(("RedirectedRoof", PropertyValue::Boolean(true))),
        ),
    ] {
        if let IfcEntity::BuildingElement {
            ifc_type,
            property_set,
            ..
        } = entity
        {
            *ifc_type = kind.into();
            if let Some((name, value)) = property {
                property_set.as_mut().unwrap().properties.push(Property {
                    name: name.into(),
                    value,
                });
            }
        }
    }
    let model = IfcModel {
        entities: vec![slab, roof, space, room, redirected_roof],
        ..Default::default()
    };
    let mesh = from_model(model, Value::Null, Vec::new()).unwrap();
    assert_eq!(mesh.metrics.element_ids, ["1", "4"]);
    assert_eq!(mesh.metrics.omitted_proxy_element_ids, ["2", "3", "5"]);
    assert_eq!(mesh.metrics.drawable_node_count, 2);
}

#[test]
fn picking_ids_are_positive_signed_revit_ids_and_distinct_from_node_counts() {
    let model = IfcModel {
        entities: vec![
            element("0007", "native_profile", true),
            element("7", "native_profile", true),
            element("0", "native_profile", true),
            element("18446744073709551615", "native_profile", true),
        ],
        ..Default::default()
    };
    let mesh = from_model(model, Value::Null, Vec::new()).unwrap();
    assert_eq!(mesh.metrics.element_ids, ["7"]);
    assert_eq!(mesh.metrics.drawable_node_count, 2);
    assert_eq!(mesh.metrics.omitted_unidentified_nodes, 2);
    assert!(
        mesh.gltf["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node.get("mesh").is_some())
            .all(|node| node["extras"]["elementId"] == "7")
    );
}

#[test]
fn glb_padding_preserves_binary_bytes_and_utf8_json_for_every_remainder() {
    for remainder in 0..4 {
        let mut mesh = fixture();
        let uri = mesh.gltf["buffers"][0]["uri"].as_str().unwrap();
        let mut binary = STANDARD
            .decode(
                uri.strip_prefix("data:application/octet-stream;base64,")
                    .unwrap(),
            )
            .unwrap();
        binary.extend(std::iter::repeat_n(0x5a, remainder));
        mesh.gltf["buffers"][0]["byteLength"] = json!(binary.len());
        mesh.gltf["buffers"][0]["uri"] = json!(format!(
            "data:application/octet-stream;base64,{}",
            STANDARD.encode(&binary)
        ));
        mesh.gltf["asset"]["generator"] = json!(format!("模型{}", "x".repeat(remainder)));
        let mut written = Vec::new();
        mesh.write_glb(&mut written).unwrap();
        let word =
            |offset| u32::from_le_bytes(written[offset..offset + 4].try_into().unwrap()) as usize;
        let json_length = word(12);
        let document: Value = serde_json::from_slice(&written[20..20 + json_length]).unwrap();
        let binary_length = word(20 + json_length);
        let binary_start = 28 + json_length;
        assert_eq!(json_length % 4, 0);
        assert_eq!(binary_length % 4, 0);
        assert_eq!(word(8), written.len());
        assert_eq!(
            &written[binary_start..binary_start + binary.len()],
            binary.as_slice()
        );
        assert!(
            written[binary_start + binary.len()..]
                .iter()
                .all(|byte| *byte == 0)
        );
        assert!(binary_length - binary.len() <= 3);
        assert_eq!(document["buffers"][0]["byteLength"], json!(binary.len()));
        assert_eq!(
            document["asset"]["generator"],
            mesh.gltf["asset"]["generator"]
        );
    }
}

#[test]
fn malformed_glb_buffer_descriptors_fail_before_any_container_bytes_are_written() {
    let mut wrong_length = fixture();
    wrong_length.gltf["buffers"][0]["byteLength"] = json!(1);
    let mut missing = fixture();
    missing.gltf["buffers"] = json!([]);
    let mut extra = fixture();
    extra.gltf["buffers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"byteLength":0}));
    let mut invalid = fixture();
    invalid.gltf["buffers"][0]["uri"] = json!("data:application/octet-stream;base64,not-base64!");
    for mesh in [wrong_length, missing, extra, invalid] {
        let mut written = Vec::new();
        assert!(mesh.write_glb(&mut written).is_err());
        assert!(written.is_empty());
    }
}

#[test]
fn atomic_mesh_write_preserves_existing_file_on_failure_and_replaces_on_success() {
    let directory = test_directory("atomic");
    let output = directory.join("模型.GLB");
    let existing = b"keep existing artifact";
    std::fs::write(&output, existing).unwrap();
    let mut invalid = fixture();
    invalid.gltf["buffers"][0]["byteLength"] = json!(1);
    assert!(invalid.write(&output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), existing);
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
    fixture().write(&output).unwrap();
    assert_eq!(&std::fs::read(&output).unwrap()[..4], b"glTF");
    let json_output = directory.join("nested").join("模型.GLTF");
    fixture().write(&json_output).unwrap();
    let gltf: Value = serde_json::from_slice(&std::fs::read(json_output).unwrap()).unwrap();
    assert_eq!(gltf["nodes"][0]["extras"]["elementId"], "9007199254740993");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn triangle_indices_of_each_unsigned_component_size_are_range_checked() {
    let (document, binary) = build_gltf(&fixture_model());
    let index = document.meshes[0].primitives[0].indices.unwrap();
    let accessor = &document.accessors[index];
    let start = document.buffer_views[accessor.buffer_view].byte_offset + accessor.byte_offset;
    let values = binary[start..start + accessor.count * 4]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| u32::from_le_bytes(*bytes))
        .collect::<Vec<_>>();
    for (component_type, width) in [(5121, 1), (5123, 2), (5125, 4)] {
        let mut document = document.clone();
        let mut binary = binary.clone();
        document.accessors[index].component_type = component_type;
        for (offset, value) in values.iter().enumerate() {
            binary[start + offset * width..start + (offset + 1) * width]
                .copy_from_slice(&value.to_le_bytes()[..width]);
        }
        assert!(document::measure(&document, &binary).is_ok());
        binary[start..start + width].fill(0xff);
        assert!(
            document::measure(&document, &binary)
                .unwrap_err()
                .to_string()
                .contains("triangle index")
        );
    }
}

#[test]
fn analytic_rectangle_fallbacks_and_partially_invalid_native_meshes_are_omitted_with_ids() {
    let mut model = fixture_model();
    let mut profile = element("3", "native_profile", true);
    if let IfcEntity::BuildingElement {
        extrusion: Some(extrusion),
        ..
    } = &mut profile
    {
        extrusion.profile_override = Some(rvt::ifc::entities::ProfileDef::ArbitraryClosed {
            points: (0..2050)
                .map(|i| {
                    let angle = i as f64 * std::f64::consts::TAU / 2050.0;
                    (angle.cos(), angle.sin())
                })
                .collect(),
        });
    }
    let mut invalid_native = element("4", "saved_graphics_mesh", false);
    if let IfcEntity::BuildingElement { solid_shape, .. } = &mut invalid_native {
        *solid_shape = Some(SolidShape::TriangulatedFaceSet {
            vertices_feet: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            triangles: vec![
                rvt::ifc::entities::BrepTriangle(0, 1, 2),
                rvt::ifc::entities::BrepTriangle(0, 1, 99),
            ],
        });
    }
    model.entities.extend([profile, invalid_native]);
    let mesh = from_model(model, Value::Null, Vec::new()).unwrap();
    assert_eq!(mesh.metrics.triangle_count, 2);
    assert_eq!(mesh.metrics.drawable_node_count, 1);
    assert_eq!(
        mesh.metrics.omitted_geometry_reasons["profile_triangulation_fallback"],
        ["3"]
    );
    assert_eq!(
        mesh.metrics.omitted_geometry_reasons["invalid_native_mesh"],
        ["4"]
    );
}

#[test]
fn unrendered_boolean_cuts_and_dropped_holes_are_reported_instead_of_filled() {
    let mut cut = element("3", "native_profile", true);
    if let IfcEntity::BuildingElement { solid_shape, .. } = &mut cut {
        *solid_shape = Some(SolidShape::BooleanResult {
            op: rvt::ifc::entities::IfcBooleanOp::Difference,
            operand_a: Box::new(SolidShape::ExtrudedArea(Extrusion::rectangle(
                4.0, 4.0, 4.0,
            ))),
            operand_b: Box::new(SolidShape::ExtrudedArea(Extrusion::rectangle(
                2.0, 2.0, 4.0,
            ))),
        });
    }
    let mut hole = element("4", "native_profile", true);
    if let IfcEntity::BuildingElement {
        extrusion: Some(extrusion),
        ..
    } = &mut hole
    {
        extrusion.profile_override = Some(rvt::ifc::entities::ProfileDef::ArbitraryWithVoids {
            points: vec![(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)],
            voids: vec![vec![(1.0, 1.0), (2.0, 1.0)]],
        });
    }
    let mut model = fixture_model();
    model.entities.extend([cut, hole]);
    let mesh = from_model(model, Value::Null, Vec::new()).unwrap();
    assert_eq!(mesh.metrics.drawable_node_count, 1);
    assert_eq!(
        mesh.metrics.omitted_geometry_reasons["unsupported_boolean_operation"],
        ["3"]
    );
    assert_eq!(
        mesh.metrics.omitted_geometry_reasons["profile_triangulation_fallback"],
        ["4"]
    );
}

#[test]
fn invalid_placements_and_compound_parts_cannot_silently_become_partial_meshes() {
    let mut invalid_placement = element("3", "native_profile", true);
    if let IfcEntity::BuildingElement {
        rotation_radians, ..
    } = &mut invalid_placement
    {
        *rotation_radians = Some(f64::NAN);
    }
    let mut compound = element("4", "native_profile", true);
    if let IfcEntity::BuildingElement { solid_shape, .. } = &mut compound {
        *solid_shape = Some(SolidShape::Compound(vec![
            SolidShape::ExtrudedArea(Extrusion::rectangle(4.0, 4.0, 4.0)),
            SolidShape::PlacedExtrusion {
                profile: rvt::ifc::entities::ProfileDef::Rectangle {
                    width_feet: 2.0,
                    depth_feet: 2.0,
                },
                origin_feet: [0.0; 3],
                axis: [0.0; 3],
                ref_direction: [1.0, 0.0, 0.0],
                depth_feet: 4.0,
            },
        ]));
    }
    let mut model = fixture_model();
    model.entities.extend([invalid_placement, compound]);
    let mesh = from_model(model, Value::Null, Vec::new()).unwrap();
    assert_eq!(mesh.metrics.drawable_node_count, 1);
    assert_eq!(
        mesh.metrics.omitted_geometry_reasons["invalid_placement"],
        ["3"]
    );
    assert_eq!(
        mesh.metrics.omitted_geometry_reasons["invalid_analytic_solid"],
        ["4"]
    );
}
