//! Numeric built-in enum identifiers only; documentation is not an input type.
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnumTable {
    by_id: BTreeMap<i64, Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnumCatalog {
    #[serde(rename = "BuiltInParameter")]
    parameters: EnumTable,
    #[serde(rename = "BuiltInCategory")]
    categories: EnumTable,
}

impl EnumCatalog {
    pub fn bundled() -> Result<Self> {
        Self::from_value(serde_json::from_str(include_str!(
            "data/revit_2024_enums.json"
        ))?)
    }
    pub fn from_value(value: Value) -> Result<Self> {
        let catalog: Self = serde_json::from_value(value)?;
        for table in [&catalog.parameters, &catalog.categories] {
            for (id, names) in &table.by_id {
                ensure!(
                    *id < 0 && !names.is_empty(),
                    "Built-in enums require negative IDs and nonempty identifier names"
                );
                for name in names {
                    let mut bytes = name.bytes();
                    ensure!(
                        bytes
                            .next()
                            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
                            && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_'),
                        "Enum names must be identifiers, not prose"
                    );
                }
            }
        }
        Ok(catalog)
    }
    pub fn parameter_name(&self, id: i64) -> Option<&str> {
        self.parameters.by_id.get(&id)?.first().map(String::as_str)
    }
    pub fn resolve(&self, id: i64) -> Option<(&str, &'static str)> {
        for (table, kind) in [
            (&self.categories, "BuiltInCategory"),
            (&self.parameters, "BuiltInParameter"),
        ] {
            if let Some(name) = table.by_id.get(&id).and_then(|names| names.first()) {
                return Some((name, kind));
            }
        }
        None
    }
}
