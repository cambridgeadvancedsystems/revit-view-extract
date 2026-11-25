use revit_view_extract::{enums::EnumCatalog, mep, output, parameters, room_federation, rtf, wire};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn typed_values_override_family_defaults_even_when_zero_blank_or_sentinel() {
    for value in [json!(0), json!(0.0), json!(""), json!(-1), json!(false)] {
        let params = BTreeMap::from([(-42, ("Integer".to_owned(), value.clone()))]);
        let family = BTreeMap::from([(-42, json!({"int":100}))]);
        assert_eq!(mep::pick_param(&params, &family, -42, "int"), value);
    }
    let family = BTreeMap::from([(-42, json!({"int":100}))]);
    assert_eq!(mep::pick_param(&BTreeMap::new(), &family, -42, "int"), 100);
}

#[test]
fn wire_ids_stay_exact_above_javascript_integer_precision() {
    let mut value = json!({"element_id":9_007_199_254_740_993i64,"owner_id":i64::MAX,
        "parameters":[{"parameter_id":9_007_199_254_740_995i64,"source_element_id":42,
            "storage_type":"ElementId","value":9_007_199_254_740_997i64},
            {"parameter_id":-1140082,"storage_type":"Double","value":480.0}],
        "upstream_ids":[42,-1],"element_ids":["9007199254740993"],"failed_element_ids":[42],
        "count":9_007_199_254_740_993i64});
    wire::encode_element_ids(&mut value).unwrap();
    assert_eq!(value["element_id"], "9007199254740993");
    assert_eq!(value["owner_id"], i64::MAX.to_string());
    assert_eq!(value["parameters"][0]["parameter_id"], "9007199254740995");
    assert_eq!(value["parameters"][0]["value"], "9007199254740997");
    assert_eq!(value["parameters"][1]["parameter_id"], -1140082);
    assert_eq!(value["parameters"][1]["value"], 480.0);
    assert_eq!(value["upstream_ids"], json!(["42", -1]));
    assert_eq!(value["count"], 9_007_199_254_740_993i64);
    let first = value.clone();
    wire::encode_element_ids(&mut value).unwrap();
    assert_eq!(value, first);
    for malformed in [json!(1.5), json!(true), json!("9223372036854775808")] {
        assert!(wire::encode_element_ids(&mut json!({"element_id":malformed})).is_err());
    }
}

#[test]
fn unicode_fallback_surrogates_and_group_state_preserve_rtf_text() {
    for (input, expected) in [
        (r"{\rtf1\uc1\u945? Beta}", "α Beta"),
        (
            r"{\rtf1 Auto \endash Internal \emdash Origin}",
            "Auto –Internal —Origin",
        ),
        (r"{\rtf1\uc1\u-10179?\u-8704?}", "😀"),
        (r"{\rtf1\uc1\u945{B}C}", "αBC"),
        (r"{\rtf1\uc1{\uc0\u945}\u946?C}", "αβC"),
        (r"{\rtf1 A{\*\unknown ignored{still ignored}}B}", "AB"),
        (r"{\rtf1\ansicpg1251 \'cf\'f0\'e8\'e2\'e5\'f2}", "Привет"),
        (r"{\rtf1 A{\pict\bin4 {}\\}B}", "AB"),
    ] {
        assert_eq!(
            rtf::strip_rtf(input.as_bytes()).unwrap(),
            expected,
            "{input}"
        );
    }
    assert!(rtf::strip_rtf(&[b'{'; 129]).is_err());
}

#[test]
fn federation_uses_oriented_room_frame_for_rotations_reflections_and_shear() {
    let rooms = vec![
        json!({"element_id":42,"number":"1","name":"Plant","level":"L1",
        "area_m2":10,"loc_ft":[0,0,0],"dims_ft":[20,2,10]}),
    ];
    for matrix in [
        [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [[1.0, 0.5, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [[2.0, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 2.0]],
    ] {
        let origin = [10.0, -20.0, 30.0];
        let point = |point| room_federation::apply_transform(point, Some(matrix), Some(origin));
        let mut items = vec![
            json!({"location_ft":point([8.0,0.0,5.0])}),
            json!({"location_ft":point([0.0,5.0,5.0]),"federated_arch_level":"stale","federated_arch_area_m2":10}),
        ];
        let report = room_federation::federate_rooms(
            &mut items,
            &rooms,
            Some(&json!({"m_3x3":matrix,"m_or":origin})),
        )
        .unwrap();
        assert_eq!(report["matched_federated_arch_room"], 1);
        assert_eq!(items[0]["federated_arch_room"], "1 Plant");
        assert!(items[1]["federated_arch_room"].is_null());
        assert!(items[1].get("federated_arch_level").is_none());
        assert!(items[1].get("federated_arch_area_m2").is_none());
    }
    for transform in [
        json!({}),
        json!({"m_3x3":[[1,0,0],[1,0,0],[0,0,1]],"m_or":[0,0,0]}),
        json!({"m_3x3":[[1,0,0,0],[0,1,0,0],[0,0,1,0],[0,0,0,1]],"m_or":[0,0,0]}),
    ] {
        assert!(room_federation::federate_rooms(&mut [], &rooms, Some(&transform)).is_err());
    }
}

#[test]
fn populated_parameter_metrics_exclude_empty_element_records() {
    let parameters = BTreeMap::from([
        (1, json!([])),
        (
            2,
            json!([
        {"parameter_id":-1140082,"name":"Current","storage_type":"Double","value":480.0,
            "converted_value":480.0,"unit":"A"},
        {"parameter_id":42,"name":"Type","storage_type":"ElementId","value":99,
            "target_name":"Symbol","target_kind":"FamilySymbol"}]),
        ),
    ]);
    let report = parameters::measure(&parameters, &json!({"parameter_records_read":2}));
    assert_eq!(report["elements_with_parameters"], 1);
    assert_eq!(report["parameter_records_read"], 2);
    assert_eq!(report["total_parameters"], 2);
    assert_eq!(report["converted_parameters"], 1);
    assert_eq!(report["resolved_pct"], 100.0);
}

#[test]
fn enum_boundary_rejects_prose_unknown_fields_and_positive_builtin_ids() {
    assert!(EnumCatalog::bundled().is_ok());
    for catalog in [
        json!({"BuiltInParameter":{"by_id":{"1":["NAME"]}},"BuiltInCategory":{"by_id":{}}}),
        json!({"BuiltInParameter":{"by_id":{"-1":["prose instead of identifier"]}},"BuiltInCategory":{"by_id":{}}}),
        json!({"BuiltInParameter":{"by_id":{},"xml_summaries":{}},"BuiltInCategory":{"by_id":{}}}),
    ] {
        assert!(EnumCatalog::from_value(catalog).is_err());
    }
}

#[test]
fn atomic_output_failure_preserves_existing_file_and_cleans_unique_temporaries() {
    let folder = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("runs")
        .join(format!("review-output-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("output.json");
    std::fs::write(&path, b"old").unwrap();
    assert!(
        output::write_atomic(&path, |writer| {
            writer.write_all(b"partial")?;
            anyhow::bail!("serialization failed")
        })
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
    output::write_atomic(&path, |writer| {
        output::write_atomic(&folder.join("nested.json"), |nested| {
            nested.write_all(b"nested")?;
            Ok(())
        })?;
        writer.write_all(b"new")?;
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 2);
    std::fs::remove_dir_all(folder).unwrap();
}
