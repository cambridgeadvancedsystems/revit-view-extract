//! Declared-element index and cross-reference lookups over native object graphs.
use crate::container::{ElemTable, read_header, scan_data_objects, visit_partitions};
use crate::graph::{Decoder, Graph, Object, extract_id};
use crate::mep::{extract_family_params, extract_param_sets, pick_param};
use crate::schema::{Registry, parse_formats};
use crate::units::{FT2_TO_M2, round};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

pub type Table = BTreeMap<i64, Value>;

struct Record {
    class_tag: u16,
    body: Vec<u8>,
}

enum CachedGraph {
    Decoded(Rc<Graph>),
    Failed(String),
}

pub struct RevitModel {
    pub name: String,
    pub registry: Registry,
    pub declared_ids: BTreeSet<i64>,
    pub covered_ids: BTreeSet<i64>,
    pub owner_map: BTreeMap<i64, i64>,
    pub by_class: BTreeMap<String, Vec<i64>>,
    records: BTreeMap<i64, Record>,
    cache: RefCell<BTreeMap<i64, CachedGraph>>,
    pub enums: crate::enums::EnumCatalog,
    pub families: Table,
    pub family_categories: BTreeMap<i64, Option<i64>>,
    pub symbols: Table,
    pub levels: Table,
    pub phases: Table,
    pub materials: Table,
    pub voltages: Table,
    pub dist_systems: Table,
    pub load_classes: Table,
    pub demand_factors: Table,
    pub param_defs: Table,
    pub spaces: Table,
    pub links: Table,
    pub views_by_id: Table,
    pub sheets: Vec<Value>,
    pub schedules: Vec<Value>,
    pub total_verified_objects: usize,
    pub load_seconds: f64,
    pub parameters: Table,
    pub parameter_metrics: Value,
}

pub fn text(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
pub fn num(value: &Value, field: &str) -> f64 {
    value.get(field).and_then(Value::as_f64).unwrap_or(0.0)
}
pub fn array<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
pub fn field_id(value: &Value, field: &str) -> Option<i64> {
    value.get(field).and_then(extract_id)
}
pub fn entry(table: &Table, id: Option<i64>) -> Value {
    id.and_then(|id| table.get(&id).cloned())
        .unwrap_or_else(|| json!({}))
}

impl RevitModel {
    pub fn from_rvt(path: &Path) -> Result<Self> {
        let started = Instant::now();
        let mut file = rvt::RevitFile::open(path).context("Open Revit input")?;
        ensure!(
            file.layout_release()? == 2024,
            "Engineering extraction supports verified Revit 2024 layouts only"
        );
        let (formats, table) = read_header(&mut file)?;
        let registry = Registry::new(parse_formats(&formats)?)?;
        let mut model = Self::empty(
            path.file_name()
                .context("Input has no filename")?
                .to_string_lossy()
                .into_owned(),
            registry,
            table,
        )?;
        visit_partitions(&mut file, |_name, partition| {
            let objects = scan_data_objects(&partition);
            model.total_verified_objects += objects.len();
            for object in objects {
                if !model.declared_ids.contains(&object.element_id) {
                    continue;
                }
                model.covered_ids.insert(object.element_id);
                if model
                    .registry
                    .classes
                    .get(&object.class_tag)
                    .is_none_or(|class| class.name != "GElement")
                {
                    model.records.insert(
                        object.element_id,
                        Record {
                            class_tag: object.class_tag,
                            body: partition[object.start + 16..object.end - 4].to_vec(),
                        },
                    );
                }
            }
            Ok(())
        })?;
        ensure!(
            model.covered_ids == model.declared_ids,
            "Incomplete verified element coverage: {} of {} declared IDs",
            model.covered_ids.len(),
            model.declared_ids.len()
        );
        for (&id, record) in &model.records {
            let name = model
                .registry
                .classes
                .get(&record.class_tag)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| format!("unk_{}", record.class_tag));
            model.by_class.entry(name).or_default().push(id);
        }
        model.build_lookups();
        let (parameters, metrics) = crate::parameters::read_native(&mut file, &model)?;
        model.parameters = parameters;
        model.parameter_metrics = metrics;
        model.load_seconds = round(started.elapsed().as_secs_f64(), 2);
        Ok(model)
    }
    fn empty(name: String, registry: Registry, table: ElemTable) -> Result<Self> {
        Ok(Self {
            name,
            registry,
            declared_ids: table.declared_ids,
            covered_ids: BTreeSet::new(),
            owner_map: table.owners,
            by_class: BTreeMap::new(),
            records: BTreeMap::new(),
            cache: RefCell::new(BTreeMap::new()),
            enums: crate::enums::EnumCatalog::bundled()?,
            families: Table::new(),
            family_categories: BTreeMap::new(),
            symbols: Table::new(),
            levels: Table::new(),
            phases: Table::new(),
            materials: Table::new(),
            voltages: Table::new(),
            dist_systems: Table::new(),
            load_classes: Table::new(),
            demand_factors: Table::new(),
            param_defs: Table::new(),
            spaces: Table::new(),
            links: Table::new(),
            views_by_id: Table::new(),
            sheets: Vec::new(),
            schedules: Vec::new(),
            total_verified_objects: 0,
            load_seconds: 0.0,
            parameters: Table::new(),
            parameter_metrics: Value::Null,
        })
    }
    pub fn set_enums(&mut self, enums: Value) -> Result<()> {
        let catalog = crate::enums::EnumCatalog::from_value(enums)?;
        let old = std::mem::replace(&mut self.enums, catalog);
        let mut parameters = self.parameters.clone();
        for parameter in parameters
            .values_mut()
            .filter_map(Value::as_array_mut)
            .flatten()
        {
            if let Err(error) = crate::parameters::enrich(self, parameter) {
                self.enums = old;
                return Err(error);
            }
        }
        self.parameter_metrics = crate::parameters::measure(&parameters, &self.parameter_metrics);
        self.parameters = parameters;
        Ok(())
    }
    pub fn decode(&self, id: i64) -> Result<Option<Rc<Graph>>> {
        if let Some(cached) = self.cache.borrow().get(&id) {
            return match cached {
                CachedGraph::Decoded(graph) => Ok(Some(Rc::clone(graph))),
                CachedGraph::Failed(error) => Err(anyhow::anyhow!(error.clone())),
            };
        }
        let Some(record) = self.records.get(&id) else {
            return Ok(None);
        };
        match Decoder::new(&record.body, &self.registry).decode() {
            Ok(graph) => {
                let graph = Rc::new(graph);
                self.cache
                    .borrow_mut()
                    .insert(id, CachedGraph::Decoded(Rc::clone(&graph)));
                Ok(Some(graph))
            }
            Err(error) => {
                self.cache
                    .borrow_mut()
                    .insert(id, CachedGraph::Failed(error.to_string()));
                Err(error)
            }
        }
    }
    pub fn decode_root(&self, id: i64) -> Result<Option<Object>> {
        if let Some(CachedGraph::Decoded(graph)) = self.cache.borrow().get(&id) {
            return Ok(graph.objects.first().cloned());
        }
        self.records
            .get(&id)
            .map(|record| Decoder::new(&record.body, &self.registry).decode_root())
            .transpose()
    }
    fn decode_diagnostics(&self) -> Value {
        let cache = self.cache.borrow();
        let mut failures = BTreeMap::<String, usize>::new();
        let mut failed_ids = Vec::new();
        let mut inexact_ids = Vec::new();
        for (id, cached) in cache.iter() {
            match cached {
                CachedGraph::Failed(_) => {
                    let class = self
                        .records
                        .get(id)
                        .and_then(|record| self.registry.classes.get(&record.class_tag))
                        .map(|class| class.name.clone())
                        .unwrap_or_else(|| "UnknownClass".to_owned());
                    *failures.entry(class).or_default() += 1;
                    failed_ids.push(id);
                }
                CachedGraph::Decoded(graph) if !graph.exact => inexact_ids.push(id),
                _ => {}
            }
        }
        json!({"attempted_graphs":cache.len(),"successful_graphs":cache.len()-failed_ids.len(),
            "failed_graphs":failed_ids.len(),"failed_by_class":failures,"failed_element_ids":failed_ids,
            "inexact_graphs":inexact_ids.len(),"inexact_element_ids":inexact_ids,
            "unattempted_semantic_records":self.records.len()-cache.len()})
    }
    pub fn safe_decode(&self, id: i64) -> Option<Rc<Graph>> {
        self.decode(id).ok().flatten()
    }
    pub fn iter_decoded(&self, class: &str) -> Vec<(i64, Rc<Graph>)> {
        self.by_class
            .get(class)
            .into_iter()
            .flatten()
            .filter_map(|id| self.safe_decode(*id).map(|graph| (*id, graph)))
            .collect()
    }
    pub fn find_instances(&self, category: i64) -> BTreeSet<i64> {
        let symbols: BTreeSet<_> = self
            .symbols
            .iter()
            .filter(|(_, s)| s["category_id"].as_i64() == Some(category))
            .map(|(id, _)| *id)
            .collect();
        self.by_class
            .get("FamilyInstance")
            .into_iter()
            .flatten()
            .filter(|id| {
                self.decode_root(**id)
                    .ok()
                    .flatten()
                    .and_then(|o| field_id(&o.fields, "m_masterSymbolId"))
                    .is_some_and(|id| symbols.contains(&id))
            })
            .copied()
            .collect()
    }
    fn build_lookups(&mut self) {
        for (id, graph) in self.iter_decoded("Family") {
            self.families
                .insert(id, json!(text(graph.root(), "m_name")));
            self.family_categories
                .insert(id, field_id(graph.root(), "m_categoryId"));
        }
        for (id, graph) in self.iter_decoded("FamilySymbol") {
            let family = field_id(graph.root(), "m_familyId");
            let params = extract_param_sets(&graph);
            let family_params = extract_family_params(&graph);
            let mut symbol = json!({"family_id":family,"family_name":family.and_then(|id| self.families.get(&id)).cloned().unwrap_or(json!("")),
                "category_id":family.and_then(|id| self.family_categories.get(&id)).copied().flatten(),
                "type_name":text(&graph.fields("SymbolInfo"),"m_name")});
            for (field, pid) in [
                ("type_mark", -1001405),
                ("description", -1010103),
                ("manufacturer", -1010108),
                ("model", -1010109),
                ("type_comments", -1010105),
            ] {
                symbol[field] = json!(
                    pick_param(&params, &family_params, pid, "str")
                        .as_str()
                        .unwrap_or("")
                        .trim()
                );
            }
            self.symbols.insert(id, symbol);
        }
        for (class, field) in [("Level", "m_text"), ("ProjectPhase", "m_name")] {
            for (id, graph) in self.iter_decoded(class) {
                let name = graph
                    .root()
                    .get(field)
                    .cloned()
                    .unwrap_or(json!(format!("{class}_{id}")));
                if class == "Level" {
                    self.levels.insert(id, name);
                } else {
                    self.phases.insert(id, name);
                }
            }
        }
        for (id, graph) in self.iter_decoded("MaterialElem") {
            let name = text(&graph.fields("Material"), "m_name");
            if !name.is_empty() {
                self.materials.insert(id, json!(name));
            }
        }
        for class in [
            "ParamElemFamily",
            "ParamElemProject",
            "ParamElemExternal",
            "ParamElemGlobal",
            "ParamElemElectricalLoadClassification",
        ] {
            for (id, graph) in self.iter_decoded(class) {
                if let Some(caption) = graph.objects.iter().find_map(|o| {
                    o.fields
                        .get("m_caption")
                        .filter(|v| truthy(v))
                        .or_else(|| o.fields.get("m_name").filter(|v| truthy(v)))
                }) {
                    self.param_defs
                        .insert(id, json!({"class":class,"caption":caption}));
                }
            }
        }
        self.electrical_lookups();
        self.space_view_lookups();
    }
    fn electrical_lookups(&mut self) {
        for (id, graph) in self.iter_decoded("RbsVoltageType") {
            let rf = graph.root();
            self.voltages.insert(id,json!({"name":text(&graph.fields("SymbolInfo"),"m_name"),
                "actual_V":round(num(rf,"m_dActualVoltage")*FT2_TO_M2,2),"min_V":round(num(rf,"m_dMinVoltage")*FT2_TO_M2,2),"max_V":round(num(rf,"m_dMaxVoltage")*FT2_TO_M2,2)}));
        }
        for (id, graph) in self.iter_decoded("RbsDistributionSysType") {
            let rf = graph.root();
            let name = text(&graph.fields("SymbolInfo"), "m_name");
            let vll = entry(&self.voltages, field_id(rf, "m_idVll"))["actual_V"].clone();
            let vlg = entry(&self.voltages, field_id(rf, "m_idVlg"))["actual_V"].clone();
            let phase = if rf["m_kPhase"] == 1 {
                "3-Phase"
            } else {
                "1-Phase"
            };
            let config = match rf["m_kConfig"].as_i64() {
                Some(0) => "Delta/1P".to_string(),
                Some(1) => "Wye".to_string(),
                Some(2) => "Delta".to_string(),
                _ => rf["m_kConfig"].to_string(),
            };
            let label = if name.is_empty() {
                format!(
                    "{}/{}V {phase} {}W {config}",
                    vlg.as_f64().unwrap_or(0.0) as i64,
                    vll.as_f64().unwrap_or(0.0) as i64,
                    rf["m_iNumWires"]
                )
            } else {
                name.clone()
            };
            self.dist_systems.insert(id,json!({"name":label,"raw_name":name,"vll_V":vll,"vlg_V":vlg,"phase":phase,"config":config,"wires":rf["m_iNumWires"]}));
        }
        for (id, graph) in self.iter_decoded("ElectricalDemandFactorDefinition") {
            self.demand_factors
                .insert(id, json!({"name":text(graph.root(),"m_name")}));
        }
        for (id, graph) in self.iter_decoded("ElectricalLoadClassification") {
            self.load_classes.insert(id,json!({"name":text(graph.root(),"m_name"),"demand_factor_id":field_id(graph.root(),"m_demandFactorId")}));
        }
    }
    fn space_view_lookups(&mut self) {
        for (id, graph) in self.iter_decoded("RoomElem") {
            let params = extract_param_sets(&graph);
            let level = field_id(graph.root(), "m_levelId")
                .filter(|id| *id != 0)
                .or_else(|| field_id(graph.root(), "m_assocLevelId"));
            let p = |id| params.get(&id).map(|p| p.1.clone()).unwrap_or(Value::Null);
            self.spaces.insert(id,json!({"element_id":id,"number":p(-1006901).as_str().unwrap_or(""),"name":p(-1006900).as_str().unwrap_or(""),"level_id":level,
                "level_name":level.and_then(|id|self.levels.get(&id)).cloned().unwrap_or(json!("")),
                "design_lighting_W":round(p(-1114230).as_f64().unwrap_or(0.0)*FT2_TO_M2,2),"design_power_W":round(p(-1114229).as_f64().unwrap_or(0.0)*FT2_TO_M2,2)}));
        }
        for (id, graph) in self.iter_decoded("RvtLinkInstance") {
            let params = extract_param_sets(&graph);
            let info = graph.fields("InstanceInfo");
            self.links.insert(id,json!({"element_id":id,"name":text(graph.root(),"m_name"),"mark":params.get(&-1001203).map(|p|p.1.clone()).unwrap_or(json!("")),
                "symbol_id":field_id(&info,"m_symbolId"),"m_3x3":info["m_Trf"]["m_3x3"],"m_or":info["m_Trf"]["m_or"]}));
        }
        for class in [
            "DBViewDrafting",
            "DBViewPlan",
            "DBViewSection",
            "DBView3d",
            "DBViewLegend",
            "DBViewSchedule",
            "PanelScheduleView",
        ] {
            for (id, graph) in self.iter_decoded(class) {
                let rf = graph.root();
                let name = text(rf, "m_viewName");
                let number = text(rf, "m_sheetNumber");
                let label = if !number.is_empty() {
                    self.sheets.push(json!({"element_id":id,"sheet_number":number,"sheet_name":name,"date":text(rf,"m_date"),"drawn_by":text(rf,"m_drawnBy"),"checked_by":text(rf,"m_checkedBy")}));
                    format!("Sheet {number}: {name}")
                } else {
                    format!("{class}: {name}")
                };
                self.views_by_id.insert(id, json!(label));
                if class == "DBViewSchedule" {
                    self.schedules.push(json!({"element_id":id,"name":name}));
                }
            }
        }
        self.sheets.sort_by_key(|s| text(s, "sheet_number"));
    }
    pub fn resolve_element_id(&self, id: i64) -> Option<(String, String)> {
        if id == -1 {
            return None;
        }
        if id < 0 {
            return self
                .enums
                .resolve(id)
                .map(|(name, kind)| (name.to_owned(), kind.to_owned()));
        }
        if let Some(symbol) = self.symbols.get(&id) {
            return Some((
                format!(
                    "{} :: {}",
                    text(symbol, "family_name"),
                    text(symbol, "type_name")
                ),
                "FamilySymbol".into(),
            ));
        }
        if let Some(voltage) = self.voltages.get(&id) {
            return Some((
                format!("{} ({} V)", text(voltage, "name"), voltage["actual_V"]),
                "RbsVoltageType".into(),
            ));
        }
        if let Some(space) = self.spaces.get(&id) {
            return Some((
                format!("{} {}", text(space, "number"), text(space, "name"))
                    .trim()
                    .into(),
                "RoomElem".into(),
            ));
        }
        for (table, kind, field) in [
            (&self.families, "Family", None),
            (&self.dist_systems, "RbsDistributionSysType", Some("name")),
            (
                &self.load_classes,
                "ElectricalLoadClassification",
                Some("name"),
            ),
            (&self.levels, "Level", None),
            (&self.phases, "ProjectPhase", None),
            (&self.materials, "MaterialElem", None),
            (&self.links, "RvtLinkInstance", Some("name")),
            (&self.views_by_id, "View", None),
        ] {
            if let Some(value) = table.get(&id) {
                return Some((
                    field
                        .map(|f| text(value, f))
                        .unwrap_or_else(|| value.as_str().unwrap_or("").into()),
                    kind.into(),
                ));
            }
        }
        if let Some(record) = self.records.get(&id) {
            let name = &self.registry.classes.get(&record.class_tag)?.name;
            Some((format!("{name}#{id}"), name.clone()))
        } else if self.covered_ids.contains(&id) {
            Some((format!("GElement#{id}"), "GElement".to_owned()))
        } else {
            None
        }
    }
    pub fn element_index(&self) -> Vec<Value> {
        self.declared_ids.iter().map(|id| {
            let class=match self.records.get(id) {
                Some(record)=>self.registry.classes.get(&record.class_tag).map(|class|class.name.as_str()).unwrap_or("UnknownClass"),
                None if self.covered_ids.contains(id)=>"GElement",
                None=>"MissingRecord",
            };
            json!({"element_id":id,"class_name":class,"owner_id":self.owner_map.get(id),"verified_record":self.covered_ids.contains(id)})
        }).collect()
    }
    pub fn select_arch_link(&self, requested: Option<i64>) -> Result<Option<&Value>> {
        if let Some(id) = requested {
            return self
                .links
                .get(&id)
                .map(Some)
                .context("Requested Architectural link ElementId was not found");
        }
        let links = self
            .links
            .iter()
            .filter(|(_, link)| text(link, "name").to_lowercase().contains("arch"))
            .collect::<Vec<_>>();
        ensure!(
            links.len() <= 1,
            "Multiple Architectural links; choose --arch-link-id from {}",
            links
                .iter()
                .map(|(id, _)| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(links.first().map(|(_, link)| *link))
    }
    pub fn extract_all(&self, rooms: Option<&[Value]>, arch_link_id: Option<i64>) -> Result<Value> {
        ensure!(
            rooms.is_some() || arch_link_id.is_none(),
            "Architectural link selection requires room data"
        );
        let mut electrical = crate::mep::extract_electrical(self);
        let mut hvac = crate::mep::extract_hvac(self);
        let notes = crate::mep::extract_text_notes(self)?;
        let mut federation = Vec::new();
        let link = rooms
            .map(|_| self.select_arch_link(arch_link_id))
            .transpose()?
            .flatten();
        if let Some(rooms) = rooms.filter(|rooms| !rooms.is_empty()) {
            federation.push((
                "federation_electrical_equipment",
                crate::room_federation::federate_rooms(
                    electrical["panels_and_equipment"].as_array_mut().unwrap(),
                    rooms,
                    link,
                )?,
            ));
            federation.push((
                "federation_mechanical_equipment",
                crate::room_federation::federate_rooms(
                    hvac["mechanical_equipment"].as_array_mut().unwrap(),
                    rooms,
                    link,
                )?,
            ));
        }
        let mut branch_total = 0;
        let mut branch_exact = 0;
        for panel in array(&electrical, "panels_and_equipment") {
            if panel["has_panel_schedule"] == true
                && num(panel, "downstream_circuit_count") > 0.0
                && num(panel, "cached_connected_kVA") > 0.0
            {
                branch_total += 1;
                if (num(panel, "cached_connected_kVA") - num(panel, "sum_direct_circuits_kVA"))
                    .abs()
                    <= 0.05
                {
                    branch_exact += 1;
                }
            }
        }
        let total = self.declared_ids.len();
        let covered = self.covered_ids.len();
        let mut result = json!({"model":self.name,"load_seconds":self.load_seconds,"elem_table_total":total,"elem_table_decoded_objects":covered,
            "elem_table_coverage_pct":if total>0 {round(covered as f64/total as f64*100.0,2)} else {0.0},
            "schema_classes_total":self.registry.classes.len(),"distinct_semantic_classes":self.by_class.len(),
            "voltage_types":self.voltages,"distribution_systems":self.dist_systems,"load_classifications":self.load_classes,"levels":self.levels,
            "native_mep_spaces_count":self.spaces.len(),"native_mep_spaces":self.spaces.values().collect::<Vec<_>>(),
            "sheets_count":self.sheets.len(),"sheets":self.sheets,"schedules_count":self.schedules.len(),"schedules":self.schedules,
            "text_notes_count":notes.len(),"text_notes":notes,"links":self.links,
            "panel_load_crosscheck":{"panels_with_schedule_and_load":branch_total,"exact_match_within_0_05_kVA":branch_exact},
            "electrical_engineering":electrical,"hvac_engineering":hvac,
            "parameters_by_element":self.parameters,"parameter_metrics":self.parameter_metrics,"element_index":self.element_index(),"decode_diagnostics":self.decode_diagnostics()});
        for (key, value) in federation {
            result[key] = value;
        }
        crate::wire::encode_element_ids(&mut result)?;
        Ok(result)
    }
}

pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.0),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architectural_link_selection_is_explicit_when_ambiguous() {
        let mut model = RevitModel::empty(
            "test.rvt".to_owned(),
            Registry::new(Vec::new()).unwrap(),
            ElemTable::default(),
        )
        .unwrap();
        assert!(model.select_arch_link(None).unwrap().is_none());
        model.links.insert(42, json!({"name":"Architectural A"}));
        assert_eq!(model.select_arch_link(None).unwrap(), model.links.get(&42));
        model.links.insert(99, json!({"name":"Architectural B"}));
        let error = model.select_arch_link(None).unwrap_err().to_string();
        assert!(error.contains("--arch-link-id"));
        assert!(error.contains("42, 99"));
        assert_eq!(
            model.select_arch_link(Some(99)).unwrap(),
            model.links.get(&99)
        );
        assert!(model.select_arch_link(Some(1)).is_err());
        assert!(model.extract_all(Some(&[]), Some(1)).is_err());
    }

    #[test]
    fn enum_replacement_rolls_back_if_existing_parameters_are_invalid() {
        let mut model = RevitModel::empty(
            "test.rvt".to_owned(),
            Registry::new(Vec::new()).unwrap(),
            ElemTable::default(),
        )
        .unwrap();
        model
            .parameters
            .insert(1, json!([{"storage_type":"Unknown"}]));
        let parameters = model.parameters.clone();
        let original = model.enums.parameter_name(-1140082).unwrap().to_owned();
        assert!(
            model
                .set_enums(json!({
                    "BuiltInParameter":{"by_id":{"-1140082":["CHANGED"]}},
                    "BuiltInCategory":{"by_id":{}}
                }))
                .is_err()
        );
        assert_eq!(
            model.enums.parameter_name(-1140082),
            Some(original.as_str())
        );
        assert_eq!(model.parameters, parameters);
    }
}
