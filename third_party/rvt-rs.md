# rvt-rs

This package uses [`rvt` 0.25.0](https://github.com/DrunkOnJava/rvt-rs/blob/eb268fdd22ced8e43bf8b2236b582f182880a2d8/Cargo.toml), pinned to commit `eb268fdd22ced8e43bf8b2236b582f182880a2d8`, under Apache-2.0. Its original [licence](rvt-rs-LICENSE) and [notice](rvt-rs-NOTICE) are preserved unchanged. Other Rust dependencies retain their own licences, listed in [the dependency inventory](rust-dependencies.json) and reproduced in [Rust notices](rust-NOTICES.txt).

Geometry follows upstream's [support profile](https://github.com/DrunkOnJava/rvt-rs/blob/eb268fdd22ced8e43bf8b2236b582f182880a2d8/docs/supported-profile.md). This adapter excludes known bounding-box stand-ins, profiles requiring a rectangular or filled-hole drawing fallback, unapplied boolean cuts/intersections and invalid geometry. Original diagnostics and omission IDs accompany each export. glTF/GLB buffers are embedded; drawable nodes retain decimal string Revit ElementIds. Coordinates are metres with Y up.

Display tessellation, some joins/openings and material appearance can differ from Revit. View visibility and graphic overrides are not read; linked 3D models are separate inputs. The pinned upstream typed-geometry API uses 32-bit ElementIds, so native geometry recovery beyond that range is not established by this adapter's 64-bit picking metadata support.
