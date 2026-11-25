use revit_view_extract::view;
use serde_json::{Value, json};
use std::path::PathBuf;

fn folder(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("runs")
        .join(format!("view-split-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn descriptor_stays_small_and_full_model_is_lossless_for_large_data() {
    let folder = folder("large");
    let descriptor = folder.join("large.json");
    let model = view::model_path(&descriptor, None).unwrap();
    assert_eq!(model, folder.join("large.model.json"));
    let mut envelope = json!({"schema_version":1,"source":{"name":"Large.rvt"},
        "engineering":{"elem_table_total":20000,"elem_table_coverage_pct":100.0,
            "element_index":(0..20000).map(|id|json!({"element_id":id})).collect::<Vec<_>>(),
            "parameters_by_element":{"1":[{"value":"x".repeat(2_000_000)}]},
            "electrical_engineering":{"circuits":[{"element_id":42}],"panel_schedules_count":2},
            "sheets_count":3,"schedules_count":4,"text_notes_count":5},
        "mesh":{"gltf_gzip_base64":"y".repeat(2_000_000),
            "metrics":{"drawable_node_count":5000,"element_ids":["42","43"]}}});
    view::write_files(&descriptor, &model, &mut envelope).unwrap();
    let bytes = std::fs::read(&descriptor).unwrap();
    assert!(bytes.len() < 10_000);
    let compact: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(compact.get("engineering").is_none());
    assert!(compact.get("mesh").is_none());
    assert_eq!(compact["kind"], "revit-view-descriptor");
    assert_eq!(compact["model_file"], "large.model.json");
    assert_eq!(compact["summary"]["elem_table_total"], 20000);
    assert_eq!(compact["summary"]["electrical_circuits"], 1);
    assert_eq!(compact["summary"]["mesh_nodes"], 5000);
    assert_eq!(compact["summary"]["drawable_element_ids"], 2);
    let full: Value = serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(&model).unwrap(),
    ))
    .unwrap();
    assert_eq!(full, envelope);
    assert_eq!(compact["document_id"], full["document_id"]);
    let first = compact["document_id"].clone();
    view::write_files(&descriptor, &model, &mut envelope).unwrap();
    assert_ne!(envelope["document_id"], first);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn derived_and_explicit_payload_collisions_fail_before_input_is_read() {
    let folder = folder("collision");
    let source = folder.join("source.rvt");
    let descriptor = folder.join("scene.json");
    let model = view::model_path(&descriptor, None).unwrap();
    std::fs::write(&source, b"untouched source").unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_revit-view-extract"))
        .arg(&source)
        .arg("--out")
        .arg(&model)
        .arg("--view-out")
        .arg(&descriptor)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Output paths must be distinct"));
    assert!(!descriptor.exists());
    assert!(!model.exists());
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_revit-view-extract"))
        .arg(&source)
        .arg("--view-out")
        .arg(&descriptor)
        .arg("--view-model-out")
        .arg(&source)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("input file"));
    assert_eq!(std::fs::read(&source).unwrap(), b"untouched source");
    assert!(view::write_files(&descriptor, &descriptor, &mut json!({})).is_err());
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn explicit_payload_path_is_used_and_requires_a_descriptor() {
    let folder = folder("explicit");
    let descriptor = folder.join("view.json");
    let explicit = folder.join("payload").join("selected.json");
    assert_eq!(
        view::model_path(&descriptor, Some(&explicit)).unwrap(),
        explicit
    );
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_revit-view-extract"))
        .arg("unused.rvt")
        .arg("--view-model-out")
        .arg(&explicit)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("--view-out"));
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
#[ignore = "requires existing large view envelopes in RVT_VIEW_FIXTURES"]
fn split_existing_live_models_without_reextracting_engineering_or_geometry() {
    let inputs =
        PathBuf::from(std::env::var_os("RVT_VIEW_FIXTURES").expect("Set RVT_VIEW_FIXTURES"));
    let outputs = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("runs")
        .join("split-views");
    std::fs::create_dir_all(&outputs).unwrap();
    let mut report = Vec::new();
    for name in ["electrical", "hvac", "architectural"] {
        let source = inputs.join(format!("{name}-view.json"));
        let mut full: Value = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(&source).unwrap(),
        ))
        .unwrap();
        let descriptor = outputs.join(format!("{name}.json"));
        let model = view::model_path(&descriptor, None).unwrap();
        view::write_files(&descriptor, &model, &mut full).unwrap();
        let compact: Value = serde_json::from_slice(&std::fs::read(&descriptor).unwrap()).unwrap();
        assert!(std::fs::metadata(&descriptor).unwrap().len() < 10_000);
        let saved: Value = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(&model).unwrap(),
        ))
        .unwrap();
        assert_eq!(saved, full);
        assert_eq!(compact["document_id"], saved["document_id"]);
        report.push(json!({"model":name,"descriptor_bytes":std::fs::metadata(&descriptor).unwrap().len(),
            "payload_bytes":std::fs::metadata(&model).unwrap().len(),"document_id":compact["document_id"],
            "model_file":compact["model_file"],"full_model_roundtrip_exact":true}));
    }
    std::fs::write(
        outputs.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
