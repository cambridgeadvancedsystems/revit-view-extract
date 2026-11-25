//! Reject upstream drawing fallbacks before they can become visible meshes.

use rvt::ifc::body_geometry::{
    Body, element_body, exact_section, extrusion_rings, profile_rings, solid_mesh,
};
use rvt::ifc::entities::{
    Extrusion, IfcBooleanOp, IfcEntity, ProfileDef, RepresentationMap, SolidShape,
};

/// The pinned drawing API may replace an untriangulatable section with its
/// rectangle or drop a hole. Exact-section preflight rejects that path. Saved
/// meshes are checked before upstream can silently filter invalid triangles.
pub(super) fn issue(entity: &IfcEntity, maps: &[RepresentationMap]) -> Option<&'static str> {
    let body = element_body(entity, maps)?;
    if let IfcEntity::BuildingElement {
        location_feet,
        rotation_radians,
        ..
    } = entity
        && (location_feet.is_some_and(|p| !p.iter().all(|v| v.is_finite()))
            || rotation_radians.is_some_and(|v| !v.is_finite()))
    {
        return Some("invalid_placement");
    }
    match body {
        Body::Extrusion(extrusion) => extrusion_issue(extrusion),
        Body::Solid(shape) => {
            let mut pending = vec![shape];
            while let Some(shape) = pending.pop() {
                let issue = match shape {
                    SolidShape::ExtrudedArea(extrusion) => extrusion_issue(extrusion),
                    SolidShape::FacetedBrep {
                        vertices_feet,
                        triangles,
                    }
                    | SolidShape::TriangulatedFaceSet {
                        vertices_feet,
                        triangles,
                    } => (triangles.is_empty()
                        || !vertices_feet
                            .iter()
                            .flatten()
                            .all(|value| value.is_finite())
                        || triangles.iter().any(|triangle| {
                            [triangle.0, triangle.1, triangle.2]
                                .iter()
                                .any(|index| *index as usize >= vertices_feet.len())
                        }))
                    .then_some("invalid_native_mesh"),
                    SolidShape::SweptPath {
                        directrix_points_feet,
                        ..
                    } if !directrix_points_feet
                        .iter()
                        .flatten()
                        .all(|v| v.is_finite()) =>
                    {
                        Some("invalid_sweep_path")
                    }
                    SolidShape::RevolvedArea { angle_radians, .. }
                    | SolidShape::PlacedRevolvedArea { angle_radians, .. }
                        if angle_radians.abs() > std::f64::consts::TAU =>
                    {
                        Some("unsupported_revolution_angle")
                    }
                    SolidShape::RevolvedArea { profile, .. }
                    | SolidShape::PlacedRevolvedArea { profile, .. }
                    | SolidShape::SweptPath { profile, .. }
                    | SolidShape::PlacedExtrusion { profile, .. } => profile_issue(profile)
                        .or_else(|| {
                            solid_mesh(shape)
                                .filter(|mesh| {
                                    !mesh.is_empty()
                                        && mesh.vertices.iter().flatten().all(|v| v.is_finite())
                                })
                                .is_none()
                                .then_some("invalid_analytic_solid")
                        }),
                    SolidShape::Compound(shapes) => {
                        pending.extend(shapes);
                        None
                    }
                    SolidShape::BooleanResult {
                        op,
                        operand_a,
                        operand_b,
                    } => {
                        if !matches!(op, IfcBooleanOp::Union) {
                            return Some("unsupported_boolean_operation");
                        }
                        pending.extend([operand_a.as_ref(), operand_b.as_ref()]);
                        None
                    }
                    _ => Some("unsupported_solid_shape"),
                };
                if issue.is_some() {
                    return issue;
                }
            }
            None
        }
        _ => Some("unsupported_body"),
    }
}

fn extrusion_issue(extrusion: &Extrusion) -> Option<&'static str> {
    if !extrusion.height_feet.is_finite() || extrusion.height_feet <= 0.0 {
        return Some("invalid_extrusion_depth");
    }
    let Some((outer, holes)) = extrusion_rings(extrusion) else {
        return Some("invalid_profile");
    };
    section_issue(&outer, &holes)
}

fn profile_issue(profile: &ProfileDef) -> Option<&'static str> {
    let Some((outer, holes)) = profile_rings(profile) else {
        return Some("invalid_profile");
    };
    section_issue(&outer, &holes)
}

fn section_issue(outer: &[(f64, f64)], holes: &[Vec<(f64, f64)>]) -> Option<&'static str> {
    // Even exact_section cleans degenerate holes before triangulating. A
    // source hole must survive that cleaning rather than become filled solid.
    exact_section(outer, holes)
        .filter(|section| section.rings.len() == holes.len() + 1)
        .is_none()
        .then_some("profile_triangulation_fallback")
}
