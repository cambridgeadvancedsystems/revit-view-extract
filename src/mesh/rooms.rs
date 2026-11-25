//! Room envelopes for spatial federation, separate from drawable geometry.

use anyhow::{Context, Result};
use rvt::RevitFile;
use rvt::ifc::entities::{IfcEntity, PropertyValue};
use rvt::ifc::{ExportOptions, SavedMeshChoice, run_export, schedule_csv::room_rows};
use serde_json::{Value, json};
use std::path::Path;

/// Recover room metadata and native record envelopes for linked-room matching.
/// Bounds are an explicit spatial-search envelope, never a displayed mesh.
/// Area is absent when the room's actual outline was not recovered.
pub fn architectural_rooms(input: &Path) -> Result<Vec<Value>> {
    let mut file = RevitFile::open(input).context("open architectural room source")?;
    let run = run_export(
        &mut file,
        &ExportOptions::default().with_saved_meshes(SavedMeshChoice::Off),
    )
    .context("recover architectural rooms")?;
    super::require_complete_read(&run.diagnostics)?;
    Ok(rooms_from_model(&run.model))
}

fn rooms_from_model(model: &rvt::ifc::IfcModel) -> Vec<Value> {
    room_rows(model)
        .into_iter()
        .filter_map(|room| {
            if room.space_kind != "room" {
                return None;
            }
            let IfcEntity::BuildingElement {
                location_feet: Some(location),
                extrusion: Some(body),
                property_set,
                ..
            } = model.entities.get(room.entity_index)?
            else {
                return None;
            };
            let height = property_set
                .iter()
                .flat_map(|set| &set.properties)
                .find_map(|property| match property.value {
                    PropertyValue::LengthFeet(height) if property.name == "BoundingBoxHeight" => {
                        Some(height)
                    }
                    _ => None,
                })
                .unwrap_or(body.height_feet);
            let dimensions = [body.width_feet, body.depth_feet, height];
            if !location
                .iter()
                .chain(dimensions.iter())
                .all(|value| value.is_finite())
                || dimensions.iter().any(|value| *value <= 0.0)
            {
                return None;
            }
            let id = room
                .revit_element_id
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0)?;
            Some(json!({
                "element_id": id,
                "number": room.number,
                "name": room.name,
                "level": room.level,
                "level_elevation_ft": room.level_elevation_feet.unwrap_or(location[2]),
                "area_m2": room.area_square_feet.map(|area| area * 0.3048 * 0.3048),
                "loc_ft": location,
                "dims_ft": dimensions,
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_envelopes_keep_original_record_height_and_do_not_invent_floor_area() {
        let entity = serde_json::from_value(json!({
            "kind": "BuildingElement", "ifc_type": "IFCSPACE", "name": "Room",
            "type_guid": "123", "location_feet": [10.0,20.0,30.0],
            "extrusion": {"width_feet": 2.0, "depth_feet": 4.0, "height_feet": 6.0},
            "property_set": {"name":"record", "properties":[
                {"name":"SpaceKind", "value":{"kind":"Text", "value":"room"}},
                {"name":"RoomNumber", "value":{"kind":"Text", "value":"101"}},
                {"name":"RoomName", "value":{"kind":"Text", "value":"Equipment"}},
                {"name":"BoundingBoxHeight", "value":{"kind":"LengthFeet", "value":8.0}}
            ]}
        }))
        .unwrap();
        let model = rvt::ifc::IfcModel {
            entities: vec![entity],
            ..Default::default()
        };
        let rooms = rooms_from_model(&model);
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0]["dims_ft"], json!([2.0, 4.0, 8.0]));
        assert_eq!(rooms[0]["loc_ft"], json!([10.0, 20.0, 30.0]));
        assert_eq!(rooms[0]["level_elevation_ft"], 30.0);
        assert_eq!(rooms[0]["area_m2"], Value::Null);
        assert_eq!(rooms[0]["number"], "101");
        assert_eq!(rooms[0]["name"], "Equipment");
    }
}
