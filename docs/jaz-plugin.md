# Using the Jaz plugin

Revit Viewer runs on the Jaz backend machine. Native ZIPs include the executable, extraction skill, model viewer and element inspector. Source and Marketplace installations include Rust source and bundled views; the skill builds the executable with Cargo on first use.

## Install

Choose the archive matching the backend's operating system and architecture from the [release assets](https://github.com/cambridgeadvancedsystems/revit-view-extract/releases). If your Jaz version offers **Settings → Plugins → Choose ZIP**, install it there. Otherwise extract the `revit-view-extract/` folder into the backend runtime's `plugins/` directory.

For Jaz's default runtime directory, macOS / Linux:

```sh
unzip ./revit-view-extract-0.1.2-aarch64-apple-darwin.zip -d "$HOME/.jaz/plugins"
chmod +x "$HOME/.jaz/plugins/revit-view-extract/bin/revit-view-extract"
```

Use the Intel macOS or Linux archive name when appropriate. Windows PowerShell:

```powershell
$revitPluginRoot = Join-Path $env:USERPROFILE '.jaz\plugins'
Expand-Archive -Path '.\revit-view-extract-0.1.2-x86_64-pc-windows-msvc.zip' -DestinationPath $revitPluginRoot
```

Substitute the backend's configured runtime root if it differs. Preserve an existing plugin folder before replacement, then reopen plugin settings or start a new agent session. No account, MCP server, Node or Python runtime is needed to use a native ZIP.

To install source, use Jaz's Git installer with `cambridgeadvancedsystems/revit-view-extract`, or install the entry from the [official catalog](https://github.com/cambridgeadvancedsystems/jaz-plugins-official). A compatible Rust toolchain and network access for pinned dependencies are needed for the first build. Jaz does not run install hooks or select a native release asset automatically.

## Extract and open a model

Ask Jaz to use `revit-view-extract:revit-view-extract` on a local Revit 2024 file. The skill writes:

| File | Purpose |
| --- | --- |
| `engineering.json` | Standalone engineering data |
| `revit-view.json` | Small index followed by Jaz |
| `revit-view.model.json` | Complete engineering data and mesh payload |

The model view is `revit-view-extract/viewer`; the property view is `revit-view-extract/inspector`. In each tab, choose **Open extracted model** and select `revit-view.model.json`. Use the model viewer to orbit, pan, zoom, pick, focus or isolate elements. Both views offer element and property search. The inspector works without WebGL.

The current Jaz thread-file bridge reads up to 1 MiB. The index stays below that limit; the full file is read locally by the browser File API. One file selection is needed whenever a tab opens. A new index invalidates the previously imported file, and the matching export ID prevents opening a stale or different model. If the backend is another machine, copy the full model file to the client before selecting it.

Revit Viewer opens on demand in thread side panels and has no left-sidebar navigation entry. Selection and search preferences are saved where Jaz provides view state; geometry and full model data are never stored in those preferences.

## Support limits

- Revit 2024 is the supported input format. Saved geometry is partial; missing meshes do not prove missing elements. Check export metrics for unsupported or omitted geometry.
- Element-table coverage measures verified declared IDs, not complete decoding of every graph or parameter. Unknown parameter IDs and values without known unit specifications remain explicit in the output.
- Large models require substantial browser memory. The list renders the first 300 search matches; search covers the complete index.
- The views bundle Three.js and decode embedded buffers without network requests. External buffers and textures are rejected. Native browser gzip decompression and WebGL are required for compressed 3D viewing; engineering inspection remains available if rendering fails.
