//! Electrical, HVAC, parameter and annotation extraction from decoded graphs.
use crate::graph::{Graph, extract_id};
use crate::model::{RevitModel, Table, array, entry, field_id, num, text, truthy};
use crate::units::{FT_TO_M, FT2_TO_M2, round};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub type Params = BTreeMap<i64, (String, Value)>;
pub type FamilyParams = BTreeMap<i64, Value>;

pub fn extract_param_sets(graph: &Graph) -> Params {
    let mut params = BTreeMap::new();
    for object in &graph.objects {
        let Some(storage) = object.class_name.strip_prefix("ParamValueSet") else {
            continue;
        };
        let storage = storage
            .replace("AString", "String")
            .replace("Int", "Integer");
        for pair in array(&object.fields, "m_paramSet") {
            if let Some(id) = field_id(pair, "m_paramId") {
                let value = if storage == "ElementId" {
                    json!(extract_id(&pair["m_value"]))
                } else {
                    pair["m_value"].clone()
                };
                params.insert(id, (storage.clone(), value));
            }
        }
    }
    params
}

pub fn extract_family_params(graph: &Graph) -> FamilyParams {
    let mut params = BTreeMap::new();
    for object in graph
        .objects
        .iter()
        .filter(|o| o.class_name == "FamilyParams")
    {
        for param in array(&object.fields, "m_params") {
            if let Some(id) = field_id(param, "m_paramId") {
                params.insert(id,json!({"str":param.get("m_str").cloned().unwrap_or(json!("")),"dbl":param.get("m_value").cloned().unwrap_or(json!(0.0)),
                    "int":param.get("m_int").cloned().unwrap_or(json!(0)),"eid":field_id(param,"m_elemId")}));
            }
        }
    }
    params
}

pub fn pick_param(params: &Params, family: &FamilyParams, id: i64, slot: &str) -> Value {
    fn usable(value: &Value) -> bool {
        truthy(value) && value.as_i64() != Some(-1)
    }
    // Typed parameter sets are authoritative, including an explicit zero/blank.
    if let Some((_, value)) = params.get(&id)
        && !value.is_null()
    {
        return value.clone();
    }
    family
        .get(&id)
        .and_then(|p| p.get(slot))
        .filter(|v| usable(v))
        .cloned()
        .unwrap_or(Value::Null)
}

fn location(graph: &Graph) -> Value {
    let info = graph.fields("InstanceInfo");
    for origin in [&info["m_Trf"]["m_or"], &graph.root()["m_instOrigin"]] {
        if origin.as_array().is_some_and(|a| a.len() == 3) {
            return origin.clone();
        }
    }
    Value::Null
}

pub fn extract_text_notes(model: &RevitModel) -> Result<Vec<Value>> {
    let mut notes = Vec::new();
    for (id, graph) in model.iter_decoded("TextNote") {
        let rtf = graph.fields("RtfData");
        let bytes = array(&rtf, "m_arrRtfStream")
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|byte| u8::try_from(byte).ok())
                    .context("Invalid RTF byte")
            })
            .collect::<Result<Vec<_>>>()?;
        let text =
            crate::rtf::strip_rtf(&bytes).with_context(|| format!("Decode TextNote {id}"))?;
        if text.is_empty() {
            continue;
        }
        let owner = field_id(graph.root(), "m_ownerDBViewId");
        let label = owner.map(|id| {
            model
                .views_by_id
                .get(&id)
                .cloned()
                .unwrap_or(json!(format!("View#{id}")))
        });
        notes.push(json!({"element_id":id,"owner_view_id":owner,"owner_view":label,"text":text}));
    }
    Ok(notes)
}

fn connectors(graph: &Graph) -> (Option<i64>, Vec<i64>) {
    let mut panel = None;
    let mut fallback = None;
    let mut devices = BTreeSet::new();
    for object in graph.objects.iter().filter(|o| o.class_name == "Connector") {
        for reference in array(&object.fields, "m_arrRefs") {
            let Some(id) = field_id(reference, "m_id").filter(|id| *id >= 0) else {
                continue;
            };
            if reference["m_connType"] == 4 && num(reference, "m_nIndex") >= 50000.0 {
                panel = Some(id);
            } else {
                if reference["m_connType"] == 4 && fallback.is_none() {
                    fallback = Some(id);
                }
                devices.insert(id);
            }
        }
    }
    let panel = panel.or(fallback);
    if let Some(id) = panel {
        devices.remove(&id);
    }
    (panel, devices.into_iter().collect())
}

fn circuits(model: &RevitModel) -> (Vec<Value>, BTreeMap<i64, Vec<Value>>) {
    let mut circuits = Vec::new();
    let mut by_panel: BTreeMap<i64, Vec<Value>> = BTreeMap::new();
    for (id, graph) in model.iter_decoded("RbsElectricalSystem") {
        let rf = graph.root();
        let (panel, devices) = connectors(&graph);
        let va = round(num(rf, "m_dApparentLoad") * FT2_TO_M2, 2);
        let record = json!({"element_id":id,"circuit_number":text(rf,"m_number"),"load_name":text(rf,"m_strDescription"),
            "load_classification":text(rf,"m_strLoadClassifications"),"panel_id":panel,"connected_device_ids":devices,
            "voltage_V":round(num(rf,"m_dVoltage")*FT2_TO_M2,2),"apparent_load_VA":va,"apparent_load_kVA":round(va/1000.0,3),
            "true_load_W":round(num(rf,"m_dTrueLoad")*FT2_TO_M2,2),"phase_A_VA":round(num(rf,"m_dApparentLoadPhaseA")*FT2_TO_M2,2),
            "phase_B_VA":round(num(rf,"m_dApparentLoadPhaseB")*FT2_TO_M2,2),"phase_C_VA":round(num(rf,"m_dApparentLoadPhaseC")*FT2_TO_M2,2),
            "rating_A":num(rf,"m_dRating"),"frame_A":num(rf,"m_dFrame"),"poles":rf.get("m_nPoles").cloned().unwrap_or(json!(0)),
            "start_slot":rf.get("m_nStartSlot").cloned().unwrap_or(json!(0)),"wire_size":text(rf,"m_strWireSize"),
            "length_m":round(num(rf,"m_dLength")*FT_TO_M,2),"voltage_drop_V":round(num(rf,"m_dVoltageDrop")*FT2_TO_M2,3)});
        if let Some(panel) = panel {
            by_panel.entry(panel).or_default().push(record.clone());
        }
        circuits.push(record);
    }
    (circuits, by_panel)
}

fn panel_schedules(model: &RevitModel) -> Table {
    let mut schedules = BTreeMap::new();
    for (id, graph) in model.iter_decoded("PanelScheduleView") {
        let rf = graph.root();
        if let Some(target) = field_id(rf, "m_targetId") {
            let data = graph.fields("PanelScheduleData");
            schedules.insert(target,json!({"schedule_view_id":id,"view_name":text(rf,"m_viewName"),"target_panel_id":target,
                "slots":data["m_numberOfSlots"],"schedule_type":data["m_scheduleType"],"out_of_date":rf["m_outOfDate"]}));
        }
    }
    schedules
}

fn analytical_nodes(model: &RevitModel) -> Vec<Value> {
    let mut nodes = Vec::new();
    for (id, graph) in model.iter_decoded("MEPAnalyticalDistributionNode") {
        let params = extract_param_sets(&graph);
        let name = params
            .get(&-1153537)
            .map(|p| p.1.clone())
            .filter(truthy)
            .unwrap_or(json!(""));
        let mut node = json!({"element_id":id,"name":name,"kind":"Unknown","voltage_V":0.0,"rating_A":null,"phases":null,
            "total_demand_kVA":0.0,"upstream_ids":[],"downstream_ids":[]});
        for object in &graph.objects {
            let fields = &object.fields;
            let spec = match object.class_name.as_str() {
                "AnalyticalPowerSourceData" => {
                    Some(("PowerSource", "m_totalDemandLoad", true, false, true))
                }
                "AnalyticalBusData" => Some(("Bus", "m_totalDemandLoad", true, true, true)),
                "AnalyticalTransformerData" => {
                    Some(("Transformer", "m_totalDemandLoad", false, false, false))
                }
                "AnalyticalTransferSwitchData" => {
                    Some(("TransferSwitch", "m_totalDemandLoad", false, false, false))
                }
                "AnalyticalEquipmentLoadData" => {
                    Some(("EquipmentLoad", "m_dApparentLoad", true, false, false))
                }
                _ => None,
            };
            if let Some((kind, load, voltage, rating, phases)) = spec {
                node["kind"] = json!(kind);
                node["total_demand_kVA"] = json!(round(num(fields, load) * FT2_TO_M2 / 1000.0, 3));
                if voltage {
                    node["voltage_V"] = json!(round(num(fields, "m_dVoltage") * FT2_TO_M2, 2));
                }
                if rating {
                    node["rating_A"] = fields["m_dRating"].clone();
                }
                if phases {
                    node["phases"] = fields["m_numberOfPhases"].clone();
                }
            } else {
                let field = match object.class_name.as_str() {
                    "AnalyticalDistributionNodeDownstreamData" => {
                        Some(("downstream_ids", "m_downstreamNodeIds"))
                    }
                    "AnalyticalSingleUpstreamNodeData" => {
                        Some(("upstream_ids", "m_upstreamNodeIds"))
                    }
                    _ => None,
                };
                if let Some((output, input)) = field {
                    node[output] = json!(
                        array(fields, input)
                            .iter()
                            .map(extract_id)
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
        nodes.push(node);
    }
    nodes
}

fn space_label(model: &RevitModel, id: Option<i64>) -> Value {
    id.and_then(|id| model.spaces.get(&id))
        .map(|space| json!(format!("{} {}", text(space, "number"), text(space, "name")).trim()))
        .unwrap_or(Value::Null)
}

fn panel_record(
    model: &RevitModel,
    id: i64,
    schedules: &Table,
    by_panel: &BTreeMap<i64, Vec<Value>>,
) -> Option<Value> {
    let graph = model.safe_decode(id)?;
    let rf = graph.root();
    let symbol = entry(&model.symbols, field_id(rf, "m_masterSymbolId"));
    let params = extract_param_sets(&graph);
    let family = extract_family_params(&graph);
    let p = |pid, slot| pick_param(&params, &family, pid, slot);
    let dpm = graph.fields("ElectricalFamInstDesignPropertyManager");
    let space = field_id(&dpm, "m_idSpace");
    let fed = by_panel.get(&id).map(Vec::as_slice).unwrap_or(&[]);
    let sum = round(fed.iter().map(|c| num(c, "apparent_load_VA")).sum(), 2);
    let mut breakdown = serde_json::Map::new();
    for load in array(&dpm, "m_LCLoadMap") {
        let lcid = field_id(load, "first");
        let name = lcid
            .and_then(|id| model.load_classes.get(&id))
            .map(|v| text(v, "name"))
            .unwrap_or_else(|| {
                format!(
                    "LC#{}",
                    lcid.map(|i| i.to_string()).unwrap_or("None".into())
                )
            });
        let connected = round(num(&load["second"], "first") * FT2_TO_M2, 2);
        let demand = round(num(&load["second"], "second") * FT2_TO_M2, 2);
        if connected > 0.0 || demand > 0.0 {
            breakdown.insert(name,json!({"connected_kVA":round(connected/1000.0,3),"demand_kVA":round(demand/1000.0,3)}));
        }
    }
    let upstream = graph
        .objects
        .iter()
        .filter(|o| o.class_name == "Connector" && o.fields["m_nIndex"] == 1)
        .flat_map(|o| array(&o.fields, "m_arrRefs"))
        .find(|r| r["m_connType"] == 4)
        .and_then(|r| field_id(r, "m_id"));
    let level = field_id(rf, "m_assocLevelId")
        .and_then(|id| model.levels.get(&id))
        .cloned()
        .unwrap_or(json!(""));
    let string_param = |pid| p(pid, "str").as_str().unwrap_or("").to_string();
    Some(
        json!({"element_id":id,"panel_name":string_param(-1140078),"mark":string_param(-1001203),"family":text(&symbol,"family_name"),"type":text(&symbol,"type_name"),
        "type_mark":text(&symbol,"type_mark"),"manufacturer":text(&symbol,"manufacturer"),"model":text(&symbol,"model"),"level":level,
        "native_space_id":space.filter(|id|*id>0),"native_space":space_label(model,space),
        "distribution_system":entry(&model.dist_systems,p(-1140064,"eid").as_i64())["name"],
        "secondary_distribution_system":entry(&model.dist_systems,p(-1140088,"eid").as_i64())["name"],
        "mains_A":p(-1140082,"dbl"),"mcb_A":p(-1140140,"dbl"),"max_poles":p(-1140079,"int"),
        "mounting":string_param(-1140081),"enclosure":string_param(-1140083),"header_notes":string_param(-1140149),
        "has_panel_schedule":schedules.contains_key(&id),"schedule_slots":schedules.get(&id).map(|v|v["slots"].clone()),
        "upstream_circuit_id":upstream,"downstream_circuit_count":fed.len(),
        "cached_connected_kVA":round(round(num(&dpm,"m_dApparentLoad")*FT2_TO_M2,2)/1000.0,3),
        "cached_demand_kVA":round(round(num(&dpm,"m_dDemandLoad")*FT2_TO_M2,2)/1000.0,3),
        "sum_direct_circuits_kVA":round(sum/1000.0,3),"load_classification_breakdown":breakdown,"location_ft":location(&graph),"bbox_m":null,
        "parameters":model.parameters.get(&id).cloned().unwrap_or(json!([]))}),
    )
}

pub fn extract_electrical(model: &RevitModel) -> Value {
    let (circuits, by_panel) = circuits(model);
    let schedules = panel_schedules(model);
    let mut ids = model.find_instances(-2001040);
    ids.extend(
        schedules
            .keys()
            .chain(by_panel.keys())
            .filter(|id| **id > 0),
    );
    let mut panels = ids
        .into_iter()
        .filter_map(|id| panel_record(model, id, &schedules, &by_panel))
        .collect::<Vec<_>>();
    let names: BTreeMap<_, _> = panels
        .iter()
        .filter(|p| truthy(&p["panel_name"]))
        .map(|p| (field_id(p, "element_id").unwrap(), p["panel_name"].clone()))
        .collect();
    let circuit_panels = circuits
        .iter()
        .filter_map(|c| Some((field_id(c, "element_id")?, field_id(c, "panel_id")?)))
        .collect::<BTreeMap<_, _>>();
    for panel in &mut panels {
        let upstream = field_id(panel, "upstream_circuit_id")
            .and_then(|id| circuit_panels.get(&id))
            .copied();
        panel["supply_from_panel_id"] = json!(upstream);
        panel["supply_from_panel"] = upstream
            .map(|id| {
                names
                    .get(&id)
                    .cloned()
                    .unwrap_or(json!(format!("Elem#{id}")))
            })
            .unwrap_or(Value::Null);
    }
    json!({"circuits":circuits,"panel_schedules_count":schedules.len(),"panels_and_equipment":panels,"analytical_distribution_nodes":analytical_nodes(model)})
}

pub fn extract_hvac(model: &RevitModel) -> Value {
    let systems = model
        .iter_decoded("RbsHvacSystem")
        .into_iter()
        .map(|(id, graph)| {
            let rf = graph.root();
            json!({"element_id":id,"name":text(rf,"m_strName"),"type_id":field_id(rf,"m_typeId"),
            "flow_CFM":rf.get("m_dFlow").and_then(Value::as_f64).map(|v|round(v*60.0,2))})
        })
        .collect::<Vec<_>>();
    let equipment = model.find_instances(-2001140).into_iter().map(|id| {
        let mut item = json!({"element_id":id,"mark":"","family":"","type":"","manufacturer":"","model":"","level":"","native_space":null,"location_ft":null,"engineering_params":{},"parameters":model.parameters.get(&id).cloned().unwrap_or(json!([]))});
        if let Some(graph) = model.safe_decode(id) {
            let symbol = entry(&model.symbols,field_id(graph.root(),"m_masterSymbolId"));
            let params = extract_param_sets(&graph);
            let family = extract_family_params(&graph);
            for (out,input) in [("family","family_name"),("type","type_name"),("manufacturer","manufacturer"),("model","model")] { item[out] = json!(text(&symbol,input)); }
            item["mark"] = json!(pick_param(&params,&family,-1001203,"str").as_str().unwrap_or(""));
            item["level"] = field_id(graph.root(),"m_assocLevelId").and_then(|id|model.levels.get(&id)).cloned().unwrap_or(json!(""));
            item["location_ft"] = location(&graph);
            for object in graph.objects.iter().filter(|o|o.class_name.contains("DesignPropertyManager")) { item["native_space"] = space_label(model,field_id(&object.fields,"m_idSpace")); }
        }
        let mut engineering=serde_json::Map::new();
        if let Some(parameters)=model.parameters.get(&id).and_then(Value::as_array) {
            for param in parameters {
                if ["V","VA","W","CFM","Pa","A"].contains(&text(param,"unit").as_str()) {
                    let name=param["name"].as_str().map(str::to_owned).unwrap_or_else(||param["parameter_id"].to_string());
                    engineering.insert(name,json!(format!("{} {}",param["converted_value"],text(param,"unit"))));
                }
            }
        }
        item["engineering_params"]=Value::Object(engineering);
        item
    }).collect::<Vec<_>>();
    json!({"hvac_systems":systems,"mechanical_equipment":equipment})
}
