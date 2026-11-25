//! Opt-in validation against the public Revit 2024 sample models.
//! RVT_TEST_MODELS points to a local directory; no files are downloaded.
use revit_view_extract::{graph::extract_id, mesh, model::RevitModel};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[test]
#[ignore = "requires the local public Electrical, HVAC and Architectural Revit 2024 fixtures"]
fn live_declared_ids_engineering_parameters_and_linked_rooms() {
    let root = PathBuf::from(
        std::env::var_os("RVT_TEST_MODELS")
            .expect("Set RVT_TEST_MODELS to the local public sample directory"),
    );
    let arch = root.join("Snowdon_Towers_Sample_Architectural.rvt");
    let rooms = mesh::architectural_rooms(&arch).unwrap();
    assert_eq!(rooms.len(), 54);
    let mut reports = Vec::new();
    let mut physical_examples = BTreeSet::new();
    for (kind, total, sheets, schedules, notes, spaces, circuits, panels, nodes, hvac, equipment) in [
        ("Electrical", 19769, 26, 39, 89, 130, 448, 53, 50, 0, 0),
        ("HVAC", 12679, 18, 22, 64, 67, 0, 0, 0, 148, 37),
        ("Architectural", 47233, 55, 92, 336, 155, 0, 0, 0, 0, 0),
    ] {
        let path = root.join(format!("Snowdon_Towers_Sample_{kind}.rvt"));
        let model = RevitModel::from_rvt(&path).unwrap();
        assert_eq!(model.declared_ids.len(), total);
        assert_eq!(model.covered_ids, model.declared_ids);
        let result = model.extract_all(Some(&rooms), None).unwrap();
        let indexed: BTreeSet<_> = result["element_index"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| extract_id(&item["element_id"]).unwrap())
            .collect();
        assert_eq!(indexed, model.declared_ids);
        for (field, expected) in [
            ("sheets_count", sheets),
            ("schedules_count", schedules),
            ("text_notes_count", notes),
            ("native_mep_spaces_count", spaces),
        ] {
            assert_eq!(result[field], json!(expected), "{kind} {field}");
        }
        assert_eq!(
            result["electrical_engineering"]["circuits"]
                .as_array()
                .unwrap()
                .len(),
            circuits
        );
        assert_eq!(
            result["electrical_engineering"]["panel_schedules_count"],
            json!(panels)
        );
        assert_eq!(
            result["electrical_engineering"]["analytical_distribution_nodes"]
                .as_array()
                .unwrap()
                .len(),
            nodes
        );
        assert_eq!(
            result["hvac_engineering"]["hvac_systems"]
                .as_array()
                .unwrap()
                .len(),
            hvac
        );
        assert_eq!(
            result["hvac_engineering"]["mechanical_equipment"]
                .as_array()
                .unwrap()
                .len(),
            equipment
        );
        assert!(!model.parameters.is_empty());
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("xml_summaries")
        );
        for parameter in model
            .parameters
            .values()
            .filter_map(Value::as_array)
            .flatten()
        {
            if let (Some(unit), Some(value)) = (
                parameter["unit"].as_str(),
                parameter["converted_value"].as_f64(),
            ) && [
                ("V", 12000.0),
                ("V", 480.0),
                ("V", 208.0),
                ("CFM", 500.0),
                ("Pa", 20.0),
            ]
            .contains(&(unit, value))
            {
                physical_examples.insert(format!("{value} {unit}"));
            }
        }
        if kind == "Electrical" {
            assert_eq!(
                result["electrical_engineering"]["panels_and_equipment"]
                    .as_array()
                    .unwrap()
                    .len(),
                200
            );
            assert_eq!(
                result["federation_electrical_equipment"]["matched_federated_arch_room"],
                166
            );
        }
        if kind == "HVAC" {
            assert_eq!(
                result["federation_mechanical_equipment"]["matched_federated_arch_room"],
                37
            );
        }
        reports.push(json!({"model":kind,"declared_ids":model.declared_ids,"covered_ids":model.covered_ids,
            "parameter_metrics":model.parameter_metrics,"sheets":sheets,"schedules":schedules,"text_notes":notes}));
    }
    for expected in ["12000 V", "480 V", "208 V", "500 CFM", "20 Pa"] {
        assert!(
            physical_examples.contains(expected),
            "Missing physical example {expected}"
        );
    }
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("runs")
        .join("live-validation");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("report.json"),
        serde_json::to_vec_pretty(&json!({"models":reports,"physical_examples":physical_examples}))
            .unwrap(),
    )
    .unwrap();
}
