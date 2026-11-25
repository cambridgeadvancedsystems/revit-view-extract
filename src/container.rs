//! CFB stream access, checksum pages, gzip framing and verified element records.
use crate::binary::{Reader, u32_at, u64_at};
use anyhow::{Context, Result, bail, ensure};
use flate2::bufread::GzDecoder;
use rvt::{RevitFile, compression::InflateLimits};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

pub const PAGE_PAYLOAD: usize = 64_896;
pub const PAGE_TRAILER: usize = 353;
const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b, 8];

#[derive(Debug, Default)]
pub struct ElemTable {
    pub class_tag: u16,
    pub stride: usize,
    pub declared_ids: BTreeSet<i64>,
    pub owners: BTreeMap<i64, i64>,
}

#[derive(Debug, Clone)]
pub struct DataObject {
    pub start: usize,
    pub end: usize,
    pub element_id: i64,
    pub class_tag: u16,
}

pub fn strip_checksum_pages(raw: &[u8]) -> Vec<u8> {
    raw.chunks(PAGE_PAYLOAD + PAGE_TRAILER)
        .flat_map(|page| page[..page.len().min(PAGE_PAYLOAD)].iter().copied())
        .collect()
}

#[derive(Debug)]
struct InflateLimit(usize);
impl std::fmt::Display for InflateLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Inflated stream exceeds {} bytes", self.0)
    }
}
impl std::error::Error for InflateLimit {}

fn inflate_member(data: &[u8], offset: usize, limit: usize) -> Result<(Vec<u8>, usize)> {
    let input = data.get(offset..).context("Gzip offset outside stream")?;
    if !input.starts_with(GZIP_MAGIC) {
        bail!("Missing gzip header at {offset}");
    }
    let mut decoder = GzDecoder::new(input);
    let mut output = Vec::new();
    decoder
        .by_ref()
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut output)?;
    if output.len() > limit {
        return Err(InflateLimit(limit).into());
    }
    let consumed = input.len() - decoder.into_inner().len();
    Ok((output, consumed))
}

pub fn inflate_gzip(raw: &[u8], offset: usize) -> Result<Vec<u8>> {
    inflate_gzip_limited(raw, offset, InflateLimits::default().max_output_bytes)
}

fn inflate_gzip_limited(raw: &[u8], offset: usize, limit: usize) -> Result<Vec<u8>> {
    let stripped = strip_checksum_pages(raw);
    match inflate_member(&stripped, offset, limit) {
        Ok((bytes, _)) => Ok(bytes),
        Err(error) if error.is::<InflateLimit>() => Err(error),
        Err(_) => inflate_member(raw, offset, limit).map(|result| result.0),
    }
}

pub fn inflate_partition(raw: &[u8]) -> Result<Vec<u8>> {
    inflate_partition_limited(raw, InflateLimits::default())
}

fn inflate_partition_limited(raw: &[u8], limits: InflateLimits) -> Result<Vec<u8>> {
    let stripped = strip_checksum_pages(raw);
    let mut output = Vec::new();
    let mut pos = 0;
    let total_limit = limits.max_stream_total_bytes_for(raw.len());
    while let Some(index) = stripped[pos..]
        .windows(3)
        .position(|word| word == GZIP_MAGIC)
    {
        pos += index;
        match inflate_member(&stripped, pos, limits.max_output_bytes) {
            Ok((bytes, consumed)) if consumed > 0 => {
                if output.len().saturating_add(bytes.len()) > total_limit {
                    return Err(InflateLimit(total_limit).into());
                }
                output.extend(bytes);
                pos += consumed;
            }
            Err(error) if error.is::<InflateLimit>() => return Err(error),
            _ => pos += 1,
        }
        if pos >= stripped.len() {
            break;
        }
    }
    Ok(output)
}

pub fn parse_elem_table(data: &[u8]) -> Result<ElemTable> {
    let mut reader = Reader::new(data);
    let class_tag = reader.u16()?;
    let count = reader.u32()? as usize;
    if count == 0 {
        return Ok(ElemTable {
            class_tag,
            stride: 40,
            ..Default::default()
        });
    }
    let mut candidates = Vec::new();
    for (stride, id_offset) in [(40usize, 20usize), (28, 0)] {
        let Some(end) = count.checked_mul(stride).and_then(|n| n.checked_add(6)) else {
            continue;
        };
        if end > data.len() {
            continue;
        }
        let mut previous = None;
        let rising = (0..count).all(|i| {
            let offset = 6 + i * stride + id_offset;
            let id = if stride == 40 {
                u64_at(data, offset)
            } else {
                u32_at(data, offset).map(u64::from)
            };
            let Some(id) = id else {
                return false;
            };
            let rising = previous.is_none_or(|previous| previous < id);
            previous = Some(id);
            rising
        });
        if rising {
            candidates.push((data.len() - end, stride));
        }
    }
    candidates.sort();
    let stride = candidates
        .first()
        .context("Unsupported Global/ElemTable layout")?
        .1;
    let mut table = ElemTable {
        class_tag,
        stride,
        ..Default::default()
    };
    for i in 0..count {
        let off = 6 + i * stride;
        let (id, owner, unset_owner) = if stride == 40 {
            let id = u64_at(data, off + 20).context("Truncated element id")?;
            let owner = u64_at(data, off + 28).context("Truncated element owner")?;
            (id, owner, owner == 0 || owner == u64::MAX)
        } else {
            let id = u32_at(data, off).context("Truncated element id")? as u64;
            let owner = u32_at(data, off + 24).context("Truncated element owner")? as u64;
            (id, owner, owner == 0 || owner == u32::MAX as u64)
        };
        let id = i64::try_from(id).context("ElementId exceeds signed 64-bit range")?;
        ensure!(
            table.declared_ids.insert(id),
            "Duplicate element id in table"
        );
        if !unset_owner {
            table.owners.insert(
                id,
                i64::try_from(owner).context("Owner ElementId exceeds signed 64-bit range")?,
            );
        }
    }
    Ok(table)
}

pub fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for block in data.chunks(5552) {
        for byte in block {
            a += *byte as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

pub fn scan_data_objects(data: &[u8]) -> Vec<DataObject> {
    let mut result = Vec::new();
    let mut start = 0usize;
    while data.len().saturating_sub(start) >= 24 {
        let size = u32_at(data, start + 12).unwrap() as usize;
        if (4..=50_000_000).contains(&size)
            && let Some(end) = start.checked_add(20).and_then(|n| n.checked_add(size))
            && end <= data.len()
            && u32_at(data, end - 4) == Some(size as u32)
            && u32_at(data, start + 8) == Some(adler32(&data[start + 16..end - 4]))
        {
            result.push(DataObject {
                start,
                end,
                element_id: i64::from_le_bytes(data[start..start + 8].try_into().unwrap()),
                class_tag: u32_at(data, start + 16).unwrap() as u16,
            });
            start = end;
        } else {
            start += 1;
        }
    }
    result
}

pub fn read_header(file: &mut RevitFile) -> Result<(Vec<u8>, ElemTable)> {
    let limit = file.limits().inflate_limits.max_output_bytes;
    let formats = inflate_gzip_limited(&file.read_stream("Formats/Latest")?, 0, limit)?;
    let raw = file.read_stream("Global/ElemTable")?;
    let bytes = inflate_gzip_limited(&raw, 8, limit).or_else(|error| {
        if error.is::<InflateLimit>() {
            Err(error)
        } else {
            inflate_gzip_limited(&raw, 0, limit)
        }
    })?;
    Ok((formats, parse_elem_table(&bytes)?))
}

pub fn visit_partitions(
    file: &mut RevitFile,
    mut partition: impl FnMut(&str, Vec<u8>) -> Result<()>,
) -> Result<()> {
    let mut paths = file
        .stream_names()
        .into_iter()
        .filter(|name| name.starts_with("Partitions/"))
        .collect::<Vec<_>>();
    paths.sort();
    let limits = file.limits().inflate_limits;
    for name in paths {
        partition(
            &name,
            inflate_partition_limited(&file.read_stream(&name)?, limits)
                .with_context(|| format!("Inflate {name}"))?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn inflated_member_limit_is_an_error_instead_of_a_skipped_partition() {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&[7; 128]).unwrap();
        let raw = encoder.finish().unwrap();
        assert!(
            inflate_gzip_limited(&raw, 0, 16)
                .unwrap_err()
                .is::<InflateLimit>()
        );
        assert!(
            inflate_partition_limited(
                &raw,
                InflateLimits {
                    max_output_bytes: 16
                }
            )
            .unwrap_err()
            .is::<InflateLimit>()
        );
    }
}
