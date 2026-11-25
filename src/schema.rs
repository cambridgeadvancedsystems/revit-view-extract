//! Forward-only parser for the self-describing Formats/Latest registry.
use crate::binary::Reader;
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub base: u8,
    pub modifier: u8,
    pub flags: u16,
    pub array_count: Option<usize>,
    pub references: Vec<u16>,
    pub nested: Option<Box<Field>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Class {
    pub tag: u16,
    pub name: String,
    pub parent_tag: u16,
    pub fields: Vec<Field>,
}

struct Parser<'a> {
    reader: Reader<'a>,
    classes: Vec<Option<Class>>,
}

impl Parser<'_> {
    fn reference(&mut self, depth: usize) -> Result<u16> {
        ensure!(depth <= 128, "Schema recursion limit");
        let raw = self.reader.u16()?;
        let tag = raw & 0x7fff;
        let next = 12 + self.classes.len();
        if raw & 0x8000 != 0 {
            ensure!(
                tag as usize == next,
                "Non-sequential schema tag {tag}, expected {next}"
            );
            self.definition(depth + 1)?;
        } else {
            ensure!((tag as usize) < next, "Forward schema reference {tag}");
        }
        Ok(tag)
    }
    fn repeated_references(&mut self, expected: &[u16], depth: usize) -> Result<Vec<u16>> {
        expected
            .iter()
            .map(|tag| {
                let actual = self.reference(depth + 1)?;
                ensure!(actual == *tag, "Nested schema reference mismatch");
                Ok(actual)
            })
            .collect()
    }
    fn field(&mut self, depth: usize) -> Result<Field> {
        ensure!(depth <= 128, "Schema recursion limit");
        let count = self.reader.u32()? as usize;
        let name = self.reader.text(count)?;
        let descriptor = self.reader.u32()?;
        let base = descriptor as u8;
        let modifier = (descriptor >> 8) as u8;
        let flags = (descriptor >> 16) as u16;
        ensure!(
            (1..=14).contains(&base)
                && [0, 1, 2, 3, 4, 16, 17, 18, 19, 80, 81, 82, 83, 84, 96].contains(&modifier),
            "Unsupported schema descriptor {descriptor:#x} for {name}"
        );
        let array_count = if (16..=19).contains(&modifier) {
            Some(self.reader.u32()? as usize)
        } else {
            None
        };
        ensure!(
            array_count.is_none_or(|count| count <= 100_000),
            "Schema fixed-array count limit"
        );
        let mut references = Vec::new();
        if base == 14 && [0, 16, 80].contains(&modifier) {
            references.push(self.reference(depth + 1)?);
        }
        let nested = if base == 13 {
            let nested = self.field(depth + 1)?;
            references.extend(&nested.references);
            for _ in 1..array_count.unwrap_or(1) {
                references.extend(self.repeated_references(&nested.references, depth + 1)?);
            }
            if array_count.is_none() && nested.base == 14 && nested.array_count.is_some() {
                references.extend(self.repeated_references(&nested.references, depth + 1)?);
            }
            Some(Box::new(nested))
        } else {
            None
        };
        Ok(Field {
            name,
            base,
            modifier,
            flags,
            array_count,
            references,
            nested,
        })
    }
    fn definition(&mut self, depth: usize) -> Result<()> {
        ensure!(depth <= 128, "Schema recursion limit");
        ensure!(self.reader.u16()? == 0, "Invalid schema definition header");
        let count = self.reader.u16()? as usize;
        let name = self.reader.text(count)?;
        let ordinal = self.classes.len();
        ensure!(12 + ordinal <= 0x7fff, "Schema class count limit");
        let tag = u16::try_from(12 + ordinal)?;
        self.classes.push(None);
        let parent_tag = self.reference(depth + 1)?;
        let _version = self.reader.u32()?;
        let count = self.reader.u32()?;
        ensure!(count <= 100_000, "Too many schema fields");
        let fields = (0..count)
            .map(|_| self.field(depth + 1))
            .collect::<Result<_>>()?;
        let opaque_count = self.reader.u32()? as usize;
        self.reader.take(
            opaque_count
                .checked_mul(16)
                .ok_or_else(|| anyhow::anyhow!("Opaque count overflow"))?,
        )?;
        self.classes[ordinal] = Some(Class {
            tag,
            name,
            parent_tag,
            fields,
        });
        Ok(())
    }
}

pub fn parse_formats(data: &[u8]) -> Result<Vec<Class>> {
    let mut parser = Parser {
        reader: Reader::new(data),
        classes: Vec::new(),
    };
    while data.len().saturating_sub(parser.reader.pos) > 8 {
        parser.definition(0)?;
    }
    ensure!(
        parser.reader.take(8)? == [0; 8] && parser.reader.pos == data.len(),
        "Invalid schema terminator"
    );
    parser
        .classes
        .into_iter()
        .map(|c| c.ok_or_else(|| anyhow::anyhow!("Incomplete schema class")))
        .collect()
}

#[derive(Clone)]
pub struct PlanField {
    pub class_name: String,
    pub field: Field,
    pub key: String,
}

pub struct Registry {
    pub classes: BTreeMap<u16, Class>,
    pub plans: BTreeMap<u16, Vec<PlanField>>,
}

impl Registry {
    pub fn new(classes: Vec<Class>) -> Result<Self> {
        let mut indexed = BTreeMap::new();
        for class in classes {
            ensure!(
                (12..=0x7fff).contains(&class.tag),
                "Invalid schema class tag"
            );
            ensure!(
                !indexed.contains_key(&class.tag),
                "Duplicate schema class tag"
            );
            indexed.insert(class.tag, class);
        }
        let classes = indexed;
        let mut registry = Self {
            classes,
            plans: BTreeMap::new(),
        };
        for tag in registry.classes.keys().copied().collect::<Vec<_>>() {
            let mut seen = BTreeSet::new();
            let mut lineage = Vec::new();
            let mut current = tag;
            while current >= 12 {
                if !seen.insert(current) {
                    bail!("Schema parent cycle at {current}");
                }
                let class = registry
                    .classes
                    .get(&current)
                    .ok_or_else(|| anyhow::anyhow!("Unknown schema parent {current}"))?;
                ensure!(lineage.len() < 128, "Schema inheritance depth limit");
                lineage.push(class);
                current = class.parent_tag;
            }
            lineage.reverse();
            let mut names = BTreeMap::new();
            for class in &lineage {
                for field in class.fields.iter().filter(|f| f.flags & 2 == 0) {
                    *names.entry(&field.name).or_insert(0) += 1;
                }
            }
            let mut plan = Vec::new();
            let mut keys = BTreeSet::new();
            for class in &lineage {
                for field in class.fields.iter().filter(|f| f.flags & 2 == 0) {
                    let key = if names[&field.name] > 1 {
                        format!("{}::{}", class.name, field.name)
                    } else {
                        field.name.clone()
                    };
                    ensure!(keys.insert(key.clone()), "Duplicate schema field key {key}");
                    plan.push(PlanField {
                        class_name: class.name.clone(),
                        field: field.clone(),
                        key,
                    });
                }
            }
            registry.plans.insert(tag, plan);
        }
        Ok(registry)
    }
}
