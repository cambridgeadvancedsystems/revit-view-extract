# View data contract

The CLI's `--view-out revit-view.json` writes a small descriptor and its sibling `revit-view.model.json`. The descriptor follows this shape:

```json
{"schema_version":1,"kind":"revit-view-descriptor","document_id":"unique-export-id","source":{"name":"model.rvt"},"model_file":"revit-view.model.json","summary":{"elem_table_total":100}}
```

`--view-model-out` can override the sibling payload path. The descriptor must stay small enough for Jaz's 1 MiB thread-file reader. The user opens the full payload through the view's browser File API; it must match the descriptor's `document_id`. Each new descriptor clears old imported data. The full payload has this shape:

```json
{
  "schema_version": 1,
  "document_id": "unique-export-id",
  "source": { "name": "model.rvt" },
  "engineering": { "electrical_engineering": {}, "hvac_engineering": {} },
  "mesh": { "gltf": { "asset": { "version": "2.0" } }, "metrics": {} }
}
```

The engineering object is the CLI engineering export, unmodified. The UI joins `parameters_by_element` to the full declared `element_index` and recursively indexes records with `element_id` or `elementId`, normalising numeric IDs to strings. Mesh-bearing glTF nodes carry `extras.elementId` as a string, linking them to engineering records. A node that has only saved mesh metadata remains inspectable but has no invented engineering values.

For meshes larger than 8 MiB of compact glTF JSON, `mesh.gltf_gzip_base64` replaces `mesh.gltf`: it contains base64-encoded gzip of the same glTF JSON, without a redundant uncompressed copy. The browser uses native `DecompressionStream("gzip")`; a current browser is required for this format. The glTF itself contains embedded base64 buffers. Its node transforms, including the root transform from feet/Z-up to metres/Y-up, are respected by GLTFLoader. A GLTFLoader `loadBufferView` adapter decodes contained buffers directly, including nonzero offsets and shared buffers, because sandbox CSP forbids data-URI fetching. External buffers, images, textures and required glTF extensions are rejected. An engineering-only JSON export also opens in the inspector.

For a thread view the value arrives as `window.data`; a navigation page supplies `{items:[{folder,data,threads}]}`. Updates arrive through `window.onData(callback)`; for current Jaz these are compact descriptors. Full payloads remain supported directly for hosts without that file limit. On replacement, old graphics resources are disposed, selection is retained only if the ID still exists, and a stale asynchronous mesh parse cannot replace a newer model. Each page item retains its associated thread link.

Element search examines every indexed record; the list renders the first 300 matches to bound DOM size. Property search matches property paths and values. Use a narrower query to reveal another element. Mesh parsing and rendering still use memory proportional to the full model: large saved caches can create large JSON files and require substantial browser memory.
