//! Schema-directed scalar decoding and FIFO deferred-pointer graph traversal.
use crate::binary::Reader;
use crate::schema::{Field, Registry};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    pub class_tag: u16,
    pub class_name: String,
    pub token: u32,
    pub start: usize,
    pub end: usize,
    pub fields: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graph {
    pub consumed: usize,
    pub total: usize,
    pub exact: bool,
    pub objects: Vec<Object>,
    pub edges: Vec<(usize, usize, u32, usize, u16)>,
}

impl Graph {
    pub fn root(&self) -> &Value {
        &self.objects[0].fields
    }
    pub fn fields(&self, name: &str) -> Value {
        self.objects
            .iter()
            .find(|o| o.class_name == name)
            .map(|o| o.fields.clone())
            .unwrap_or_else(|| json!({}))
    }
}

pub struct Decoder<'a> {
    reader: Reader<'a>,
    registry: &'a Registry,
    pending: Vec<(u32, u16, usize)>,
    values: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(body: &'a [u8], registry: &'a Registry) -> Self {
        Self {
            reader: Reader::new(body),
            registry,
            pending: Vec::new(),
            values: 0,
        }
    }
    fn pointer(&mut self, external: bool) -> Result<Value> {
        let offset = self.reader.pos;
        let token = self.reader.u32()?;
        let tag = if token != 0 && !external {
            let tag = self.reader.u16()?;
            ensure!(
                self.registry.classes.contains_key(&tag),
                "Unknown pointer class {tag}"
            );
            self.pending.push((token, tag, offset));
            Some(tag)
        } else {
            None
        };
        Ok(json!({"_ptr":token,"_tag":tag,"_off":offset}))
    }
    fn class(&mut self, tag: u16, depth: usize) -> Result<Value> {
        ensure!(depth <= 64, "Graph recursion limit");
        let plan = self.registry.plans.get(&tag).context("Unknown class tag")?;
        let mut fields = Map::new();
        for step in plan {
            let field = &step.field;
            let value = match (
                step.class_name.as_str(),
                field.name.as_str(),
                field.base,
                field.modifier,
            ) {
                ("ClassDefinitionRef", "m_ref", 10, 0) => {
                    let class_tag = self.reader.u16()?;
                    json!({"class_tag":class_tag,"class_name":self.registry.classes.get(&class_tag).map(|c| &c.name)})
                }
                ("ESEntity", "m_blob", 14, 1) => {
                    let discriminator = self.reader.u32()? as i32;
                    ensure!(
                        discriminator == 0,
                        "Populated Extensible Storage is unsupported"
                    );
                    json!({"_es":null})
                }
                ("VarSketchObj", "m_params", 14, 0x54) => {
                    self.field(field, 0x51, field.array_count, depth + 1)?
                }
                _ => self.field(field, field.modifier, field.array_count, depth + 1)?,
            };
            fields.insert(step.key.clone(), value);
        }
        Ok(Value::Object(fields))
    }
    fn scalar(&mut self, field: &Field, depth: usize) -> Result<Value> {
        Ok(match field.base {
            1 => {
                let byte = self.reader.u8()?;
                ensure!(byte <= 1, "Invalid boolean");
                json!(byte != 0)
            }
            2 => json!(self.reader.u8()?),
            3 => json!(i16::from_le_bytes(self.reader.take(2)?.try_into()?)),
            4 => json!(self.reader.u32()? as i32),
            5 => json!(self.reader.u32()?),
            6 => {
                let value = f32::from_le_bytes(self.reader.take(4)?.try_into()?);
                ensure!(value.is_finite(), "Non-finite graph float");
                json!(value)
            }
            7 => {
                let value = f64::from_le_bytes(self.reader.take(8)?.try_into()?);
                ensure!(value.is_finite(), "Non-finite graph double");
                json!(value)
            }
            9 => {
                let bytes = self.reader.take(16)?;
                let a = u32::from_le_bytes(bytes[0..4].try_into()?);
                let b = u16::from_le_bytes(bytes[4..6].try_into()?);
                let c = u16::from_le_bytes(bytes[6..8].try_into()?);
                let tail: String = bytes[8..].iter().map(|x| format!("{x:02x}")).collect();
                json!(format!(
                    "{a:08x}-{b:04x}-{c:04x}-{}-{}",
                    &tail[..4],
                    &tail[4..]
                ))
            }
            11 => json!(i64::from_le_bytes(self.reader.take(8)?.try_into()?)),
            13 => {
                let nested = field.nested.as_ref().context("Missing nested schema")?;
                self.field(nested, nested.modifier, nested.array_count, depth + 1)?
            }
            14 => self.class(
                *field
                    .references
                    .first()
                    .context("Missing inline class reference")?,
                depth + 1,
            )?,
            base => bail!("Unsupported scalar base {base}"),
        })
    }
    fn field(
        &mut self,
        field: &Field,
        modifier: u8,
        count: Option<usize>,
        depth: usize,
    ) -> Result<Value> {
        ensure!(depth <= 64, "Field recursion limit");
        self.values += 1;
        ensure!(self.values <= 500_000, "Graph value limit");
        if [1, 2, 3].contains(&modifier) && field.base == 14 {
            return self.pointer(modifier == 3);
        }
        if modifier == 0x60 && field.base == 8 {
            let count = self.reader.u32()? as usize;
            ensure!(count <= 1_000_000, "String limit");
            let bytes = self.reader.take(count * 2)?;
            let units: Vec<_> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            return Ok(json!(String::from_utf16_lossy(&units)));
        }
        if [0x10, 0x11, 0x12, 0x13, 0x50, 0x51, 0x52, 0x53].contains(&modifier) {
            let count = if modifier < 0x50 {
                count.context("Missing fixed container count")?
            } else {
                self.reader.u32()? as usize
            };
            ensure!(count <= 100_000, "Container count limit");
            return Ok(Value::Array(
                (0..count)
                    .map(|_| self.field(field, modifier & 15, None, depth + 1))
                    .collect::<Result<_>>()?,
            ));
        }
        if modifier == 0 {
            return self.scalar(field, depth);
        }
        bail!("Unsupported field modifier {modifier} base {}", field.base)
    }
    pub fn decode_root(mut self) -> Result<Object> {
        let tag = self.reader.u16()?;
        let name = self
            .registry
            .classes
            .get(&tag)
            .context("Unknown root class")?
            .name
            .clone();
        let start = self.reader.pos;
        let fields = self.class(tag, 0)?;
        Ok(Object {
            class_tag: tag,
            class_name: name,
            token: 0,
            start,
            end: self.reader.pos,
            fields,
        })
    }
    pub fn decode(mut self) -> Result<Graph> {
        let tag = self.reader.u16()?;
        ensure!(
            self.registry.classes.contains_key(&tag),
            "Unknown root class {tag}"
        );
        let mut objects = Vec::new();
        let mut edges = Vec::new();
        let mut shared = BTreeMap::new();
        let mut queue = VecDeque::from([(0, tag)]);
        while let Some((token, tag)) = queue.pop_front() {
            ensure!(objects.len() < 20_000, "Graph object limit");
            let start = self.reader.pos;
            let pending_start = self.pending.len();
            let class_name = self
                .registry
                .classes
                .get(&tag)
                .context("Unknown queued class")?
                .name
                .clone();
            let fields = self.class(tag, 0)?;
            let index = objects.len();
            objects.push(Object {
                class_tag: tag,
                class_name,
                token,
                start,
                end: self.reader.pos,
                fields,
            });
            for &(token, tag, offset) in &self.pending[pending_start..] {
                let target = if let Some((target, known_tag)) = shared.get(&token) {
                    ensure!(*known_tag == tag, "Shared graph pointer changes class");
                    *target
                } else {
                    let target = objects.len() + queue.len();
                    ensure!(target < 20_000, "Graph object queue limit");
                    queue.push_back((token, tag));
                    if token != u32::MAX {
                        shared.insert(token, (target, tag));
                    }
                    target
                };
                edges.push((index, offset, token, target, tag));
            }
        }
        Ok(Graph {
            consumed: self.reader.pos,
            total: self.reader.data.len(),
            exact: self.reader.pos == self.reader.data.len(),
            objects,
            edges,
        })
    }
}

pub fn extract_id(value: &Value) -> Option<i64> {
    let mut current = value;
    for _ in 0..64 {
        if let Some(id) = current
            .as_i64()
            .or_else(|| current.as_str().and_then(|s| s.parse().ok()))
        {
            return Some(id);
        }
        for field in ["m_id64", "m_id32"] {
            if let Some(id) = current.get(field).and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            }) {
                return Some(id);
            }
        }
        current = current.get("m_id")?;
    }
    None
}
