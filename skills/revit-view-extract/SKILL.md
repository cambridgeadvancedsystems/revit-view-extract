---
name: revit-view-extract
description: Extract engineering data and saved meshes from local Revit 2024 RVT files; inspect circuits, panels, HVAC equipment, rooms, sheets and element properties in an interactive local viewer. Use when opening or analysing an RVT model or producing a linked model and property view in Jaz.
---

# Revit Viewer

Keep source models and derived data local. Never upload them or add them to a repository. This reader supports Revit 2024; do not imply that every Revit version, family or saved geometry cache is supported. Read the CLI's diagnostics and distinguish extracted facts from unavailable data.

## Run

Resolve this skill's plugin root two directories above this file. Prefer its native release binary at `bin/revit-view-extract` on macOS/Linux or `bin/revit-view-extract.exe` on Windows. Do not run an incompatible executable. ZIP releases are platform-specific. If a trusted extracted Unix package lost its executable bit, copy the binary into the current thread's `runs/revit-bin/` folder and `chmod +x` that local copy before running it; never change laptop settings or native agent configuration.

A Git/source installation has no prebuilt binary. If a compatible binary is absent and Cargo is available, build the installed source with `cargo build --release --locked --manifest-path <plugin-root>/Cargo.toml --target-dir ./runs/revit-build`, inside the thread working folder. The executable then lives at `./runs/revit-build/release/revit-view-extract` (or `.exe`). Building requires the pinned open-source Rust dependencies; extraction itself makes no network requests. If Cargo is unavailable, ask for the matching platform ZIP. Jaz does not execute install hooks or select a platform binary from a Git repository.

Run the binary with the supplied local path, shell-quoted safely:

```text
<binary> <input.rvt> --out engineering.json --view-out revit-view.json
```

Optional outputs and federation:

```text
<binary> <input.rvt> --out engineering.json --mesh-gltf runs/model.gltf --view-out revit-view.json --arch-rvt <architectural.rvt>
```

The `--view-out` path is a small index JSON; the CLI also writes the full engineering-and-mesh payload to its sibling `<stem>.model.json`. For `revit-view.json`, this is `revit-view.model.json`. The index contains an export ID, the full filename and summary counts. Keep the index's filename plain and directly inside the thread working folder. Store scratch output under `runs/`. Do not place large model data in tool messages or chat.

## Show the result in Jaz

Call the available `show_view` tool with:

```json
{"view":"revit-view-extract/viewer","title":"Revit model","file":"revit-view.json"}
```

Open the property inspector with the same file:

```json
{"view":"revit-view-extract/inspector","title":"Element properties","file":"revit-view.json"}
```

These are plugin views, not generic artifacts. Jaz's current thread-file API truncates files at 1 MiB, so always show the small index, never the full payload. In each tab, the user selects the sibling file with **Open extracted model**; its browser File API loads the complete payload locally. Link both output files in the reply and explain that one local file selection is required each time the tab opens. The selected full file must be accessible on the client machine; if extraction runs on another backend, the user must first copy that full file to the client. File-following updates refresh the index and clear any old imported model; the user reopens the new export. The plugin navigation page discovers threads after a file-backed view has been shown. The viewer supports orbit, pan, zoom, element picking, name/ID/value search, category filters, focus, isolation and wireframe. The inspector works even when WebGL is unavailable. It only presents properties actually in the extracted data or saved mesh metadata.

## Report

State the engineering coverage, mesh coverage and omissions from the actual output. Saved meshes are a partial cache; an absent mesh does not prove an element is absent. Mesh-only IDs can have no extracted engineering properties. Do not invent ratings, manufacturer/model fields, rooms, circuit connections or physical values. A saved mesh is not a rendered 2D sheet or a Revit formula evaluator. Include the output file and both views; keep a concise explanation of any limits relevant to the user's question.
