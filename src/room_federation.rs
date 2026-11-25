//! Linked architectural-room bounding-box federation in native feet coordinates.
use crate::model::text;
use crate::units::{FT_TO_M, round};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

pub fn load_arch_rooms(elements_path: &Path, csv_path: &Path) -> Result<Vec<Value>> {
    let mut csv = csv::Reader::from_path(csv_path)?;
    let headers = csv.headers()?.clone();
    let mut metadata = BTreeMap::new();
    for row in csv.records() {
        let row = row?;
        let map: BTreeMap<_, _> = headers
            .iter()
            .zip(row.iter())
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let id: i64 = map
            .get("revit_element_id")
            .context("Missing CSV revit_element_id")?
            .parse()?;
        metadata.insert(id, map);
    }
    let elements: Vec<Value> = serde_json::from_slice(&std::fs::read(elements_path)?)?;
    let mut rooms = Vec::new();
    for element in elements.iter().filter(|e| e["class_name"] == "Room") {
        let fields: BTreeMap<_, _> = element["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| Some((f["name"].as_str()?, f.get("value")?.clone())))
            .collect();
        let coordinates: Option<Vec<f64>> = [
            "m_locationX",
            "m_locationY",
            "m_locationZ",
            "m_bboxWidth",
            "m_bboxDepth",
            "m_bboxHeight",
        ]
        .iter()
        .map(|key| fields.get(key)?.as_f64())
        .collect();
        let Some(c) = coordinates else {
            continue;
        };
        let id = crate::graph::extract_id(&element["id"]).context("Invalid room id")?;
        let meta = metadata.get(&id);
        let get = |key: &str| {
            meta.and_then(|m| m.get(key))
                .map(String::as_str)
                .unwrap_or("")
        };
        let field = |key: &str, fallback: &str| {
            fields
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(fallback)
                .to_string()
        };
        let elevation = get("level_elevation_m")
            .parse::<f64>()
            .ok()
            .map(|v| v / FT_TO_M)
            .unwrap_or(c[2]);
        rooms.push(json!({"element_id":id,"number":field("m_number",get("number")),"name":field("m_name",get("name")),"level":get("level"),
            "level_elevation_ft":elevation,"area_m2":get("area_m2").parse::<f64>().ok(),"loc_ft":&c[..3],"dims_ft":&c[3..]}));
    }
    Ok(rooms)
}

pub fn apply_transform(
    point: [f64; 3],
    matrix: Option<[[f64; 3]; 3]>,
    origin: Option<[f64; 3]>,
) -> [f64; 3] {
    let (Some(matrix), Some(origin)) = (matrix, origin) else {
        return point;
    };
    std::array::from_fn(|r| origin[r] + (0..3).map(|c| matrix[r][c] * point[c]).sum::<f64>())
}

fn vector(value: &Value) -> Option<[f64; 3]> {
    let values = value.as_array()?;
    if values.len() != 3 {
        return None;
    }
    let vector = [
        values[0].as_f64()?,
        values[1].as_f64()?,
        values[2].as_f64()?,
    ];
    vector.iter().all(|v| v.is_finite()).then_some(vector)
}

struct RoomFrame {
    inverse: [[f64; 3]; 3],
    origin: [f64; 3],
    tolerance_scale: [f64; 3],
}

impl RoomFrame {
    fn from_link(link: Option<&Value>) -> Result<Self> {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let Some(link) = link else {
            return Ok(Self {
                inverse: identity,
                origin: [0.0; 3],
                tolerance_scale: [1.0; 3],
            });
        };
        let rows = link["m_3x3"]
            .as_array()
            .filter(|rows| rows.len() == 3)
            .context("Architectural link must contain a 3x3 transform")?;
        let matrix = [
            vector(&rows[0]).context("Invalid link transform row")?,
            vector(&rows[1]).context("Invalid link transform row")?,
            vector(&rows[2]).context("Invalid link transform row")?,
        ];
        let origin =
            vector(&link["m_or"]).context("Architectural link must contain a finite origin")?;
        let scale = matrix
            .iter()
            .flatten()
            .fold(0.0f64, |scale, value| scale.max(value.abs()));
        ensure!(scale > 0.0, "Singular architectural link transform");
        let m = matrix.map(|row| row.map(|value| value / scale));
        let cofactors = [
            [
                m[1][1] * m[2][2] - m[1][2] * m[2][1],
                m[1][2] * m[2][0] - m[1][0] * m[2][2],
                m[1][0] * m[2][1] - m[1][1] * m[2][0],
            ],
            [
                m[0][2] * m[2][1] - m[0][1] * m[2][2],
                m[0][0] * m[2][2] - m[0][2] * m[2][0],
                m[0][1] * m[2][0] - m[0][0] * m[2][1],
            ],
            [
                m[0][1] * m[1][2] - m[0][2] * m[1][1],
                m[0][2] * m[1][0] - m[0][0] * m[1][2],
                m[0][0] * m[1][1] - m[0][1] * m[1][0],
            ],
        ];
        let determinant = (0..3)
            .map(|column| m[0][column] * cofactors[0][column])
            .sum::<f64>();
        ensure!(
            determinant.abs() > f64::EPSILON,
            "Singular or ill-conditioned architectural link transform"
        );
        let inverse: [[f64; 3]; 3] = std::array::from_fn(|row| {
            std::array::from_fn(|column| cofactors[column][row] / determinant / scale)
        });
        let tolerance_scale = inverse.map(|row| row[0].hypot(row[1]).hypot(row[2]));
        ensure!(
            inverse
                .iter()
                .flatten()
                .chain(tolerance_scale.iter())
                .all(|v| v.is_finite()),
            "Invalid architectural inverse transform"
        );
        Ok(Self {
            inverse,
            origin,
            tolerance_scale,
        })
    }
    fn local_point(&self, point: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|row| {
            (0..3)
                .map(|column| self.inverse[row][column] * (point[column] - self.origin[column]))
                .sum()
        })
    }
}

struct RoomBounds<'a> {
    room: &'a Value,
    center: [f64; 3],
    dimensions: [f64; 3],
}

impl<'a> RoomBounds<'a> {
    fn parse(room: &'a Value) -> Option<Self> {
        let center = vector(&room["loc_ft"])?;
        let dimensions = vector(&room["dims_ft"])?;
        dimensions.iter().all(|v| *v > 0.0).then_some(Self {
            room,
            center,
            dimensions,
        })
    }
    fn contains(&self, point: [f64; 3], scale: [f64; 3]) -> bool {
        point[0] >= self.center[0] - self.dimensions[0] / 2.0 - scale[0]
            && point[0] <= self.center[0] + self.dimensions[0] / 2.0 + scale[0]
            && point[1] >= self.center[1] - self.dimensions[1] / 2.0 - scale[1]
            && point[1] <= self.center[1] + self.dimensions[1] / 2.0 + scale[1]
            && point[2] >= self.center[2] - 1.5 * scale[2]
            && point[2] <= self.center[2] + self.dimensions[2] + scale[2]
    }
    fn footprint(&self) -> f64 {
        self.dimensions[0] * self.dimensions[1]
    }
}

pub fn federate_rooms(items: &mut [Value], rooms: &[Value], link: Option<&Value>) -> Result<Value> {
    let frame = RoomFrame::from_link(link)?;
    let bounds = rooms
        .iter()
        .filter_map(RoomBounds::parse)
        .collect::<Vec<_>>();
    let mut native = 0;
    let mut matched = 0;
    let mut both = 0;
    let mut agree = 0;
    for item in items.iter_mut() {
        let native_label = text(item, "native_space");
        if !native_label.is_empty() {
            native += 1;
        }
        let fields = item
            .as_object_mut()
            .context("Federated equipment must be an object")?;
        fields.remove("federated_arch_level");
        fields.remove("federated_arch_area_m2");
        fields.insert("federated_arch_room".to_owned(), Value::Null);
        let Some(point) = vector(&item["location_ft"]) else {
            continue;
        };
        let point = frame.local_point(point);
        let best = bounds
            .iter()
            .filter(|bounds| bounds.contains(point, frame.tolerance_scale))
            .min_by(|a, b| a.footprint().total_cmp(&b.footprint()));
        let Some(best) = best else {
            continue;
        };
        let room = best.room;
        item["federated_arch_room"] =
            json!(format!("{} {}", text(room, "number"), text(room, "name")).trim());
        item["federated_arch_level"] = room["level"].clone();
        item["federated_arch_area_m2"] = room["area_m2"].clone();
        matched += 1;
        if !native_label.is_empty() {
            both += 1;
            let native_number = native_label
                .split_whitespace()
                .next()
                .unwrap_or("")
                .split('-')
                .next()
                .unwrap_or("");
            let number = text(room, "number");
            let arch_number = number.split('-').next().unwrap_or("");
            let name = text(room, "name");
            if (!native_number.is_empty()
                && !arch_number.is_empty()
                && native_number.eq_ignore_ascii_case(arch_number))
                || (!name.is_empty() && native_label.to_lowercase().contains(&name.to_lowercase()))
            {
                agree += 1;
            }
        }
    }
    let total = items.len().max(1) as f64;
    Ok(
        json!({"total_items":items.len(),"matched_native_mep_space":native,"matched_native_pct":round(native as f64/total*100.0,2),
        "matched_federated_arch_room":matched,"matched_federated_arch_pct":round(matched as f64/total*100.0,2),
        "cross_validated_both_present":both,"cross_validation_agreement":agree,"cross_validation_agreement_pct":round(agree as f64/both.max(1) as f64*100.0,2),
        "rooms_with_valid_bounds":bounds.len(),"rooms_without_valid_bounds":rooms.len()-bounds.len()}),
    )
}
