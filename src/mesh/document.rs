//! Validate the emitted triangle buffers and measure their scene placements.

use anyhow::{Context, Result, ensure};
use rvt::ifc::gltf::{Accessor, GltfDocument};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

#[derive(Debug)]
pub(super) struct Measurement {
    pub triangle_count: usize,
    pub vertex_count: usize,
    pub primitive_count: usize,
    pub drawable_node_count: usize,
    pub bounds: Bounds,
}

const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

pub(super) fn measure(document: &GltfDocument, binary: &[u8]) -> Result<Measurement> {
    let scene = document
        .scene
        .and_then(|index| document.scenes.get(index))
        .context("glTF has no scene")?;
    let mut result = Measurement {
        triangle_count: 0,
        vertex_count: 0,
        primitive_count: 0,
        drawable_node_count: 0,
        bounds: Bounds {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        },
    };
    let mut visited = vec![false; document.nodes.len()];
    let mut pending = scene
        .nodes
        .iter()
        .map(|&index| (index, IDENTITY))
        .collect::<Vec<_>>();
    while let Some((index, parent)) = pending.pop() {
        let node = document
            .nodes
            .get(index)
            .context("glTF node index is out of bounds")?;
        ensure!(
            !visited[index],
            "glTF scene reuses a node or contains a cycle"
        );
        visited[index] = true;
        let local = node
            .matrix
            .map(|matrix| matrix.map(f64::from))
            .unwrap_or(IDENTITY);
        ensure!(
            local.iter().all(|value| value.is_finite()),
            "non-finite glTF transform"
        );
        ensure!(
            [local[3], local[7], local[11], local[15]] == [0.0, 0.0, 0.0, 1.0],
            "glTF node transform must be affine"
        );
        let world = multiply(parent, local);
        pending.extend(node.children.iter().map(|&index| (index, world)));
        let Some(mesh_index) = node.mesh else {
            continue;
        };
        let mesh = document
            .meshes
            .get(mesh_index)
            .context("glTF mesh index is out of bounds")?;
        ensure!(
            !mesh.primitives.is_empty(),
            "drawable glTF mesh has no primitives"
        );
        result.drawable_node_count += 1;
        for primitive in &mesh.primitives {
            ensure!(
                primitive.mode == 4,
                "mesh primitive must be a triangle list"
            );
            let position_index = primitive
                .attributes
                .get("POSITION")
                .context("mesh primitive has no positions")?;
            let positions = document
                .accessors
                .get(*position_index)
                .context("position accessor is out of bounds")?;
            ensure!(
                positions.component_type == 5126 && positions.type_ == "VEC3",
                "positions must be float VEC3"
            );
            let points = accessor_bytes(document, binary, positions, 12, 4)?;
            ensure!(positions.count > 0, "mesh has no vertices");
            // Bounds describe referenced geometry. Saved display buffers may
            // contain unused vertices that are not part of any triangle.
            let mut used = vec![primitive.indices.is_none(); positions.count];
            let index_count = if let Some(index) = primitive.indices {
                let accessor = document
                    .accessors
                    .get(index)
                    .context("index accessor is out of bounds")?;
                ensure!(accessor.type_ == "SCALAR", "mesh indices must be scalar");
                let width = match accessor.component_type {
                    5121 => 1,
                    5123 => 2,
                    5125 => 4,
                    _ => anyhow::bail!("mesh indices must be unsigned integers"),
                };
                let bytes = accessor_bytes(document, binary, accessor, width, width)?;
                for encoded in bytes.chunks_exact(width) {
                    let value = match width {
                        1 => u32::from(encoded[0]),
                        2 => u32::from(u16::from_le_bytes([encoded[0], encoded[1]])),
                        _ => u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]),
                    };
                    ensure!(
                        (value as usize) < positions.count,
                        "triangle index exceeds its vertex count"
                    );
                    used[value as usize] = true;
                }
                accessor.count
            } else {
                positions.count
            };
            ensure!(
                index_count > 0 && index_count % 3 == 0,
                "triangle index count is invalid"
            );
            for (point, used) in points.as_chunks::<12>().0.iter().zip(used) {
                let point = [
                    component(point, 0),
                    component(point, 4),
                    component(point, 8),
                ];
                ensure!(
                    point.iter().all(|value| value.is_finite()),
                    "mesh has a non-finite position"
                );
                if !used {
                    continue;
                }
                let transformed = transform(world, point);
                ensure!(
                    transformed.iter().all(|value| value.is_finite()),
                    "mesh has a non-finite world position"
                );
                for (axis, value) in transformed.into_iter().enumerate() {
                    result.bounds.min[axis] = result.bounds.min[axis].min(value);
                    result.bounds.max[axis] = result.bounds.max[axis].max(value);
                }
            }
            result.triangle_count = result
                .triangle_count
                .checked_add(index_count / 3)
                .context("triangle count overflow")?;
            result.vertex_count = result
                .vertex_count
                .checked_add(positions.count)
                .context("vertex count overflow")?;
            result.primitive_count += 1;
        }
    }
    ensure!(
        document
            .nodes
            .iter()
            .enumerate()
            .all(|(index, node)| node.mesh.is_none() || visited[index]),
        "mesh node is not reachable in the scene"
    );
    Ok(result)
}

fn accessor_bytes<'a>(
    document: &GltfDocument,
    binary: &'a [u8],
    accessor: &Accessor,
    width: usize,
    component_width: usize,
) -> Result<&'a [u8]> {
    let view = document
        .buffer_views
        .get(accessor.buffer_view)
        .context("glTF buffer view is out of bounds")?;
    ensure!(
        view.buffer == 0 && document.buffers.len() == 1,
        "mesh requires one embedded buffer"
    );
    ensure!(
        document.buffers[0].byte_length == binary.len(),
        "mesh buffer length mismatch"
    );
    let view_end = view
        .byte_offset
        .checked_add(view.byte_length)
        .context("buffer view length overflow")?;
    ensure!(
        view_end <= binary.len(),
        "buffer view exceeds the binary buffer"
    );
    ensure!(
        view.byte_offset.is_multiple_of(component_width)
            && accessor.byte_offset.is_multiple_of(component_width),
        "mesh accessor is not aligned to its component size"
    );
    let length = accessor
        .count
        .checked_mul(width)
        .context("accessor length overflow")?;
    let end_in_view = accessor
        .byte_offset
        .checked_add(length)
        .context("accessor offset overflow")?;
    ensure!(
        end_in_view <= view.byte_length,
        "accessor exceeds its buffer view"
    );
    let start = view
        .byte_offset
        .checked_add(accessor.byte_offset)
        .context("buffer offset overflow")?;
    let end = start
        .checked_add(length)
        .context("buffer length overflow")?;
    binary
        .get(start..end)
        .context("mesh accessor exceeds the binary buffer")
}

fn component(bytes: &[u8], offset: usize) -> f64 {
    f64::from(f32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

fn multiply(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    let mut matrix = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            matrix[column * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[column * 4 + k]).sum();
        }
    }
    matrix
}

fn transform(matrix: [f64; 16], point: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|row| {
        matrix[row] * point[0]
            + matrix[4 + row] * point[1]
            + matrix[8 + row] * point[2]
            + matrix[12 + row]
    })
}
