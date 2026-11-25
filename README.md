# Revit Viewer

Read Revit 2024 engineering data and saved mesh geometry locally, without Revit or a cloud upload. The Rust CLI extracts circuits, panels, electrical and HVAC equipment, levels, sheets, schedules, notes and links. A Jaz plugin adds an interactive Three.js model viewer and searchable element properties.

## Use the CLI

```sh
cargo build --release --locked
./target/release/revit-view-extract model.rvt --out engineering.json --view-out revit-view.json
```

Add `--mesh-gltf runs/model.gltf` for a standalone embedded-buffer glTF. Add `--arch-rvt architectural.rvt` to federate engineering equipment with architectural rooms. Run `--help` for the other inputs and outputs.

The engineering JSON keeps element IDs and physical values. The `--view-out` JSON is a compact index. Its sibling `revit-view.model.json` holds the complete engineering data and glTF, with a matching export ID. Geometry uses saved meshes and supported analytic profiles; omitted geometry is reported, not replaced with invented boxes. This is a Revit 2024 reader, not a complete Revit renderer, formula evaluator or 2D drawing export.

## Install in Jaz

Use the platform ZIP matching the **backend machine**. If the installed Jaz build offers Settings → Plugins → Choose ZIP, install it there; otherwise extract its `revit-view-extract/` folder into the backend runtime's `plugins/` directory. It includes the native executable and both self-contained views. No Node/Python runtime, account, MCP server or CDN is required for extraction and viewing.

Direct Git installation supplies source and bundled views. It requires Cargo for the first build; Jaz does not build plugins automatically. See [installation guide](docs/jaz-plugin.md).

Ask Jaz to use `revit-view-extract:revit-view-extract` on a local RVT file. The skill creates the small `revit-view.json` index plus the full `revit-view.model.json` in the thread folder and opens the index in:

- `revit-view-extract/viewer`: orbit, pan, zoom, mesh picking, focus, isolation, wireframe and searchable elements.
- `revit-view-extract/inspector`: searchable properties and category filters, including when WebGL is unavailable.

**Select `revit-view.model.json` with Open extracted model in each tab.** This local file selection bypasses the current Jaz thread-file API's 1 MiB limit; the complete model never travels through that bridge. A new export invalidates the old imported data. If the backend is another machine, copy the full file to the client first.

Revit Viewer opens on demand in thread side panels and has no left-sidebar navigation entry. Extraction stays on the backend machine; selecting a full model file reads it on the client without uploading it. The UI rejects external mesh assets and performs no runtime network requests.

## Develop

```sh
npm ci
npm run check
cargo test --locked --all-targets
npm run preview
```

Edit `views/src/`, then `npm run build` to regenerate the committed `views/viewer.html` and `views/inspector.html`. Three.js is bundled into each HTML view because Jaz loads view source into a sandbox without package-relative filesystem access. The preview uses a synthetic model.

See [release archives](docs/releasing.md) and [the view data contract](docs/view-data.md). Apache-2.0; third-party notices are in NOTICE.
