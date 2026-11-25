# Revit Viewer 0.1.2

Revit Viewer opens on demand in thread side panels, with no left-sidebar navigation entry. The model viewer, element inspector, extraction skill, display name and icon remain available. Repository, package, CLI and schema identifiers remain compatible.

Local Revit 2024 engineering and saved-mesh extraction, with a Jaz model viewer and element property inspector.

Choose the ZIP matching the Jaz backend's operating system and architecture. ZIPs include the native executable, self-contained Three.js views, skill and icon. Verify the adjacent SHA-256 file before installation. Git/source installation requires a Rust build. Open the generated small view index in Jaz, then select its sibling full model JSON with Open extracted model; the current host requires one local selection per tab.

Saved geometry is partial; missing geometry and omitted fallback proxy bodies are reported in the export. This release does not support every Revit version, evaluate arbitrary Revit formulas or render 2D drawings.
