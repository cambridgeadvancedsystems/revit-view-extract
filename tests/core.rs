use revit_view_extract::container::{
    PAGE_PAYLOAD, PAGE_TRAILER, adler32, inflate_gzip, inflate_partition, parse_elem_table,
    scan_data_objects, strip_checksum_pages,
};
use revit_view_extract::graph::{Decoder, extract_id};
use revit_view_extract::room_federation::{apply_transform, federate_rooms};
use revit_view_extract::rtf::strip_rtf;
use revit_view_extract::schema::{Class, Field, Registry, parse_formats};
use revit_view_extract::units::{FT2_TO_M2, convert_by_spec, convert_parameter, round};
use serde_json::{Value, json};
use std::io::Write;

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    writer.write_all(data).unwrap();
    writer.finish().unwrap()
}

#[test]
fn checksum_pages_and_embedded_gzip_members_preserve_payload() {
    let mut paged = vec![3u8; PAGE_PAYLOAD];
    paged.extend(vec![99u8; PAGE_TRAILER]);
    paged.extend([7, 8, 9]);
    let stripped = strip_checksum_pages(&paged);
    assert_eq!(stripped.len(), PAGE_PAYLOAD + 3);
    assert_eq!(&stripped[PAGE_PAYLOAD..], &[7, 8, 9]);
    let mut members = vec![8u8; 12];
    members.extend(gzip(b"first"));
    members.extend([0, 0, 1]);
    members.extend(gzip(b"second"));
    assert_eq!(inflate_partition(&members).unwrap(), b"firstsecond");
    let gz = gzip(b"complete gzip");
    assert_eq!(inflate_gzip(&gz, 0).unwrap(), b"complete gzip");
    assert!(inflate_gzip(&gz[..gz.len() - 3], 0).is_err());
}

#[test]
fn object_scanner_rejects_bad_checksum_and_truncation() {
    let body = [12u8, 0, 0, 0, 2, 3, 4, 5];
    let size = body.len() as u32;
    let mut object = Vec::new();
    object.extend(42u32.to_le_bytes());
    object.extend(0u32.to_le_bytes());
    object.extend(adler32(&body).to_le_bytes());
    object.extend(size.to_le_bytes());
    object.extend(body);
    object.extend(size.to_le_bytes());
    let found = scan_data_objects(&object);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].element_id, 42);
    assert_eq!(found[0].class_tag, 12);
    object[20] ^= 1;
    assert!(scan_data_objects(&object).is_empty());
    assert!(scan_data_objects(&object[..18]).is_empty());
}

#[test]
fn element_table_detects_64_and_32_bit_record_layouts() {
    for stride in [40, 28] {
        let mut data = vec![0u8; 6 + 2 * stride];
        data[0..2].copy_from_slice(&12u16.to_le_bytes());
        data[2..6].copy_from_slice(&2u32.to_le_bytes());
        for (i, id) in [21u32, 42].into_iter().enumerate() {
            let start = 6 + i * stride;
            data[start..start + 4].copy_from_slice(&id.to_le_bytes());
            if stride == 40 {
                data[start + 20..start + 24].copy_from_slice(&id.to_le_bytes());
                data[start + 28..start + 36].copy_from_slice(&7u64.to_le_bytes());
            } else {
                data[start + 24..start + 28].copy_from_slice(&7u32.to_le_bytes());
            }
        }
        let table = parse_elem_table(&data).unwrap();
        assert_eq!(table.stride, stride);
        assert_eq!(
            table.declared_ids.into_iter().collect::<Vec<_>>(),
            vec![21, 42]
        );
        assert_eq!(table.owners[&42], 7);
    }
    assert!(parse_elem_table(&[0; 5]).is_err());
}

fn scalar(name: &str, base: u8, modifier: u8) -> Field {
    Field {
        name: name.into(),
        base,
        modifier,
        flags: 0,
        array_count: None,
        references: Vec::new(),
        nested: None,
    }
}

#[test]
fn schema_parser_consumes_real_definition_framing_and_rejects_forward_reference() {
    let mut bytes = Vec::new();
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(4u16.to_le_bytes());
    bytes.extend(b"Root");
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(5u32.to_le_bytes());
    bytes.extend(b"value");
    bytes.extend(4u32.to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    bytes.extend([0; 8]);
    let classes = parse_formats(&bytes).unwrap();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].fields[0].name, "value");
    bytes[8..10].copy_from_slice(&99u16.to_le_bytes());
    assert!(parse_formats(&bytes).is_err());
}

#[test]
fn graph_uses_fifo_pointers_shared_tokens_inheritance_and_64_bit_ids() {
    let root = Class {
        tag: 12,
        name: "Root".into(),
        parent_tag: 0,
        fields: vec![
            scalar("one", 14, 1),
            scalar("same", 14, 1),
            scalar("external", 14, 3),
        ],
    };
    let child = Class {
        tag: 13,
        name: "Child".into(),
        parent_tag: 0,
        fields: vec![scalar("m_id64", 11, 0)],
    };
    let registry = Registry::new(vec![root, child]).unwrap();
    let mut body = Vec::new();
    body.extend(12u16.to_le_bytes());
    for _ in 0..2 {
        body.extend(5u32.to_le_bytes());
        body.extend(13u16.to_le_bytes());
    }
    body.extend(99u32.to_le_bytes());
    body.extend(9_000_000_001i64.to_le_bytes());
    let graph = Decoder::new(&body, &registry).decode().unwrap();
    assert!(graph.exact);
    assert_eq!(graph.objects.len(), 2);
    assert_eq!(graph.edges[0].3, graph.edges[1].3);
    assert_eq!(extract_id(&graph.objects[1].fields), Some(9_000_000_001));
    assert!(extract_id(&json!(true)).is_none());
    body.truncate(body.len() - 1);
    assert!(Decoder::new(&body, &registry).decode().is_err());
}

#[test]
fn inherited_duplicate_names_are_qualified_and_parent_cycles_are_rejected() {
    let base = Class {
        tag: 12,
        name: "Base".into(),
        parent_tag: 0,
        fields: vec![scalar("value", 4, 0)],
    };
    let child = Class {
        tag: 13,
        name: "Child".into(),
        parent_tag: 12,
        fields: vec![scalar("value", 4, 0)],
    };
    let registry = Registry::new(vec![base, child]).unwrap();
    let mut body = 13u16.to_le_bytes().to_vec();
    body.extend(7u32.to_le_bytes());
    body.extend(9u32.to_le_bytes());
    let graph = Decoder::new(&body, &registry).decode().unwrap();
    assert_eq!(graph.root()["Base::value"], 7);
    assert_eq!(graph.root()["Child::value"], 9);
    assert!(
        Registry::new(vec![Class {
            tag: 12,
            name: "Loop".into(),
            parent_tag: 12,
            fields: Vec::new()
        }])
        .is_err()
    );
}

#[test]
fn fksa_units_preserve_electrical_hvac_and_decimal_rounding_values() {
    for volts in [12000.0, 480.0, 208.0] {
        let c = convert_by_spec(
            "autodesk.spec.aec.electrical:potential-1.0.0",
            volts / FT2_TO_M2,
        )
        .unwrap();
        assert_eq!(c.value, volts);
        assert_eq!(c.unit, "V");
    }
    assert_eq!(
        convert_by_spec("autodesk.spec.aec.hvac:airFlow-1.0.0", 500.0 / 60.0)
            .unwrap()
            .value,
        500.0
    );
    assert_eq!(
        convert_by_spec("autodesk.spec.aec.hvac:pressure-1.0.0", 20.0 * 0.3048)
            .unwrap()
            .value,
        20.0
    );
    assert_eq!(convert_parameter(None, -1140082, 225.0).unwrap().unit, "A");
    assert!(convert_by_spec("unknown", 42.0).is_none());
    assert!(convert_by_spec("autodesk.spec.aec:length", f64::NAN).is_none());
    assert_eq!(round(5.9225, 3), 5.923);
    assert_eq!(round(2.675, 2), 2.67);
}

#[test]
fn rtf_nested_destinations_escapes_and_text_lines() {
    let raw = br"{\rtf1{\fonttbl{\f0 Arial;}}Hello \b world\b0\par A\'b0\tab \{safe\}}";
    assert_eq!(strip_rtf(raw).unwrap(), "Hello world\nA° {safe}");
}

#[test]
fn room_federation_applies_link_transform_and_selects_smallest_footprint() {
    assert_eq!(
        apply_transform(
            [2.0, 3.0, 4.0],
            Some([[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]),
            Some([10.0, 0.0, 1.0])
        ),
        [7.0, 2.0, 5.0]
    );
    let rooms = vec![
        json!({"element_id":1,"number":"100","name":"Large","level":"L1","area_m2":40,"loc_ft":[0,0,0],"dims_ft":[20,20,10]}),
        json!({"element_id":2,"number":"101","name":"Plant","level":"L1","area_m2":10,"loc_ft":[0,0,0],"dims_ft":[5,5,10]}),
    ];
    let mut items = vec![
        json!({"location_ft":[10,0,1],"native_space":"101 Plant"}),
        json!({"location_ft":null}),
    ];
    let report = federate_rooms(
        &mut items,
        &rooms,
        Some(&json!({"m_3x3":[[1,0,0],[0,1,0],[0,0,1]],"m_or":[10,0,0]})),
    )
    .unwrap();
    assert_eq!(items[0]["federated_arch_room"], "101 Plant");
    assert_eq!(items[1]["federated_arch_room"], Value::Null);
    assert_eq!(report["matched_federated_arch_pct"], 50.0);
    assert_eq!(report["cross_validation_agreement_pct"], 100.0);
}

#[test]
fn bundled_enums_only_contain_numeric_maps_without_documentation() {
    let raw = include_str!("../src/data/revit_2024_enums.json");
    let enums: Value = serde_json::from_str(raw).unwrap();
    assert!(!raw.contains("xml_summaries"));
    for (kind, count) in [("BuiltInParameter", 3544), ("BuiltInCategory", 1197)] {
        let table = enums[kind]["by_id"].as_object().unwrap();
        assert_eq!(table.len(), count);
        assert!(table.iter().all(|(id, names)| id.parse::<i64>().is_ok()
            && names.as_array().unwrap().iter().all(Value::is_string)));
    }
}

#[test]
fn cli_output_validation_refuses_source_aliases_and_publishes_json_atomically() {
    let folder = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("runs")
        .join(format!("output-test-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let source = folder.join("source.rvt");
    let output = folder.join("result.json");
    std::fs::write(&source, b"original").unwrap();
    assert!(revit_view_extract::output::validate(&[&source], &[&source]).is_err());
    assert!(revit_view_extract::output::validate(&[&source], &[&output, &output]).is_err());
    #[cfg(unix)]
    {
        let alias = folder.join("alias.json");
        let _ = std::fs::remove_file(&alias);
        std::fs::hard_link(&source, &alias).unwrap();
        assert!(revit_view_extract::output::validate(&[&source], &[&alias]).is_err());
    }
    revit_view_extract::output::validate(&[&source], &[&output]).unwrap();
    revit_view_extract::output::write_json(&output, &json!({"value":42})).unwrap();
    let result: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(result["value"], 42);
    assert_eq!(std::fs::read(&source).unwrap(), b"original");
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn schema_rejects_unbounded_fixed_arrays_before_repeating_references() {
    let mut bytes = Vec::new();
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(4u16.to_le_bytes());
    bytes.extend(b"Root");
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(5u32.to_le_bytes());
    bytes.extend(b"value");
    bytes.extend(0x1004u32.to_le_bytes());
    bytes.extend(u32::MAX.to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    bytes.extend([0; 8]);
    assert!(
        parse_formats(&bytes)
            .unwrap_err()
            .to_string()
            .contains("count limit")
    );
}
