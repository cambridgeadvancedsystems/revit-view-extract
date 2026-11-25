import { normalizeModels, buildElementIndex, elementLabel, elementCategory, filterElements, filterProperties, restoreState } from './data.js';
import { decodeMesh } from './mesh-data.js';
import { MeshViewer } from './scene.js';

function element(document, tag, text, attributes = {}) {
  const node = document.createElement(tag);
  if (text != null) node.textContent = String(text);
  for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
  return node;
}

export function mountApp(document, bridge, createViewer = (container, onSelect) => new MeshViewer(container, onSelect)) {
  const byId = id => document.getElementById(id);
  const state = restoreState(bridge.viewState);
  let models = [];
  let active = null;
  let content = null;
  let meshReady = false;
  let disposed = false;
  let records = new Map();
  let viewer = null;
  let loadGeneration = 0;
  const save = () => bridge.setViewState?.({ ...state, modelKey: active?.key });
  const updateScene = () => viewer?.update(state);

  function setStatus(message, error = false) {
    byId('mesh-status').textContent = message;
    byId('mesh-status').classList.toggle('error', error);
  }

  function renderProperties() {
    const record = records.get(state.selectedId);
    byId('selected-name').textContent = record ? elementLabel(record) : 'Select an element';
    byId('selected-meta').textContent = record ? `${elementCategory(record)} · ID ${record.element_id}` : 'Pick a mesh or choose an element below.';
    const rows = record ? filterProperties(record, state.propertyQuery) : [];
    byId('property-count').textContent = record ? `${rows.length} properties` : '';
    const body = byId('properties');
    body.replaceChildren();
    for (const [key, value] of rows) {
      const row = element(document, 'tr');
      row.append(element(document, 'th', key, { scope: 'row' }), element(document, 'td', value));
      body.append(row);
    }
    byId('property-empty').hidden = rows.length > 0;
    byId('property-empty').textContent = record ? 'No properties match this search.' : 'Element properties will appear here.';
    byId('focus').disabled = !record?.has_mesh || !meshReady;
    byId('isolate').disabled = !record?.has_mesh || !meshReady;
    byId('isolate').setAttribute('aria-pressed', String(state.isolate));
  }

  function select(id) {
    if (disposed || !records.has(id)) return;
    state.selectedId = id;
    if (!records.get(id).has_mesh) state.isolate = false;
    for (const row of byId('elements').children) row.setAttribute('aria-pressed', String(row.getAttribute('data-element-id') === id));
    renderProperties();
    updateScene();
    save();
  }

  function renderList() {
    const filtered = filterElements(records, state.query, state.category);
    byId('element-count').textContent = `${filtered.length} of ${records.size} elements`;
    const list = byId('elements');
    list.replaceChildren();
    // Keep the DOM bounded for large models. Search always runs across every record.
    for (const record of filtered.slice(0, 300)) {
      const row = element(document, 'button', null, { type: 'button', class: 'element-row', 'aria-pressed': String(state.selectedId === record.element_id), 'data-element-id': record.element_id });
      row.append(element(document, 'span', elementLabel(record), { class: 'element-label' }));
      row.append(element(document, 'span', `${record.element_id} · ${elementCategory(record)}${record.has_mesh ? ' · 3D' : ''}`, { class: 'element-detail' }));
      row.addEventListener('click', () => select(record.element_id));
      list.append(row);
    }
    byId('element-empty').hidden = filtered.length > 0;
    byId('element-empty').textContent = content?.descriptor
      ? `Open ${active.modelFile} to load this model's elements and properties.`
      : 'No matching elements. Clear your filters or open an export.';
    byId('list-limit').hidden = filtered.length <= 300;
  }

  function refreshElements(model, gltf, complete = true) {
    records = buildElementIndex(model?.engineering, gltf);
    if (complete && !model?.descriptor && !records.has(state.selectedId)) state.selectedId = null;
    const categories = [...new Set([...records.values()].map(elementCategory))].sort();
    const category = byId('category');
    category.replaceChildren(element(document, 'option', 'All categories', { value: '' }));
    for (const value of categories) category.append(element(document, 'option', value, { value }));
    if (!categories.includes(state.category)) state.category = '';
    category.value = state.category;
    byId('source-name').textContent = model?.name ?? 'Open a local model';
    byId('choose-json').textContent = active?.descriptor ? 'Open extracted model' : 'Open JSON';
    byId('source-summary').textContent = model?.descriptor
      ? `${Number(model.summary?.elem_table_total ?? 0).toLocaleString()} elements · select the full model export`
      : model ? `${records.size.toLocaleString()} indexed elements${model.gltf || model.gltfGzip ? ' · saved mesh geometry' : ' · engineering data'}` : 'Engineering data and saved geometry, together.';
    renderList();
    renderProperties();
  }

  async function activate(model, payload = model) {
    if (disposed) return;
    const generation = ++loadGeneration;
    active = model;
    content = payload;
    meshReady = false;
    viewer?.dispose();
    viewer = null;
    state.isolate = false;
    records = new Map();
    const thread = model?.threads?.[0];
    byId('open-thread').hidden = !thread;
    byId('open-thread').onclick = () => thread && bridge.openLink?.(`/sessions/${encodeURIComponent(thread.id)}`);
    byId('fit').disabled = true;
    byId('wireframe').disabled = true;
    try {
      refreshElements(payload, payload?.gltf, !payload?.gltfGzip);
    } catch (error) {
      renderList();
      renderProperties();
      setStatus(`Could not read model: ${error.message}`, true);
      return;
    }
    if (payload?.descriptor) {
      setStatus(`Open ${model.modelFile} to explore geometry and properties.`);
      return;
    }
    if (!(payload?.gltf || payload?.gltfGzip)) {
      setStatus(model ? 'No mesh loaded. Engineering properties are available below.' : 'Choose a JSON export, or attach a Revit file to extract.');
      return;
    }
    setStatus('Loading saved geometry…');
    try {
      const gltf = await decodeMesh(payload);
      if (generation !== loadGeneration) return;
      refreshElements(payload, gltf);
      if (document.body.dataset.view === 'inspector') {
        setStatus(`${records.size.toLocaleString()} elements · engineering and saved mesh properties`);
        return;
      }
      const nextViewer = createViewer(byId('viewport'), select);
      viewer = nextViewer;
      await nextViewer.load(gltf);
      if (generation !== loadGeneration) return;
      meshReady = true;
      nextViewer.update(state);
      const count = (gltf.nodes ?? []).filter(node => node.mesh != null).length;
      setStatus(`${count.toLocaleString()} mesh nodes · drag to orbit · scroll to zoom`);
      byId('fit').disabled = false;
      byId('wireframe').disabled = false;
      renderProperties();
    } catch (error) {
      if (generation !== loadGeneration) return;
      viewer?.dispose();
      viewer = null;
      meshReady = false;
      renderProperties();
      const unavailable = document.body.dataset.view === 'inspector' ? 'Mesh metadata unavailable' : '3D unavailable';
      setStatus(`${unavailable}: ${error.message}. Element properties remain available.`, true);
    }
  }

  function receive(payload) {
    if (disposed) return;
    const imported = active?.descriptor && content !== active;
    const previous = active?.key ?? state.modelKey;
    models = normalizeModels(payload);
    const selectModel = byId('model');
    selectModel.replaceChildren();
    for (const model of models) selectModel.append(element(document, 'option', model.name, { value: model.key }));
    selectModel.hidden = models.length < 2;
    const model = models.find(item => item.key === previous) ?? models[0] ?? null;
    selectModel.value = model?.key ?? '';
    void activate(model);
    if (imported && model?.descriptor) setStatus(`Model updated. Open ${model.modelFile} again to load the new export.`);
  }

  byId('model').addEventListener('change', event => {
    void activate(models.find(model => model.key === event.target.value));
  });
  byId('element-search').value = state.query;
  byId('element-search').addEventListener('input', event => {
    state.query = event.target.value;
    renderList();
    save();
  });
  byId('category').addEventListener('change', event => {
    state.category = event.target.value;
    renderList();
    save();
  });
  byId('property-search').value = state.propertyQuery;
  byId('property-search').addEventListener('input', event => {
    state.propertyQuery = event.target.value;
    renderProperties();
    save();
  });
  byId('fit').addEventListener('click', () => viewer?.fit());
  byId('focus').addEventListener('click', () => viewer?.fit(state.selectedId));
  byId('isolate').addEventListener('click', () => {
    state.isolate = !state.isolate;
    renderProperties();
    updateScene();
    save();
  });
  byId('show-all').addEventListener('click', () => {
    state.isolate = false;
    renderProperties();
    updateScene();
    save();
  });
  byId('wireframe').checked = state.wireframe;
  byId('wireframe').addEventListener('change', event => {
    state.wireframe = event.target.checked;
    updateScene();
    save();
  });
  byId('other-view').addEventListener('click', () => {
    const inspector = document.body.dataset.view !== 'inspector';
    bridge.openView?.(inspector ? 'inspector' : 'viewer', inspector ? 'Element properties' : 'Revit model');
  });
  byId('other-view').hidden = typeof bridge.openView !== 'function' || Array.isArray(bridge.data?.items);
  byId('choose-json').addEventListener('click', () => byId('json-file').click());
  byId('json-file').addEventListener('change', async event => {
    if (disposed) return;
    const input = event.target;
    const file = input.files?.[0];
    if (!file) return;
    const requested = active?.descriptor ? active : null;
    void activate(requested);
    const generation = loadGeneration;
    setStatus('Reading model file…');
    try {
      const value = JSON.parse(await file.text());
      if (generation !== loadGeneration) return;
      const [model] = normalizeModels(value);
      if (!model || model.descriptor) throw new Error('Choose the full model JSON, rather than its index');
      if (requested?.descriptor) {
        if (typeof requested.documentId !== 'string' || model.documentId !== requested.documentId) {
          throw new Error(`This file does not match the current export. Choose ${requested.modelFile}`);
        }
        await activate(requested, model);
      } else {
        receive(value);
      }
    } catch (error) {
      if (generation !== loadGeneration) return;
      setStatus(`Could not open JSON: ${error.message}. Choose the full model JSON export.`, true);
    } finally {
      input.value = '';
    }
  });
  byId('choose-rvt').addEventListener('click', () => byId('rvt-file').click());
  byId('rvt-file').addEventListener('change', event => {
    const files = Array.from(event.target.files ?? []);
    if (files.length) bridge.sendPrompt?.('Use the revit-view-extract skill to extract this local Revit file, create the compact revit-view.json index and its full model JSON, and open the viewer and element inspector.', files);
    event.target.value = '';
  });
  byId('choose-rvt').hidden = typeof bridge.sendPrompt !== 'function';
  byId('viewport').addEventListener('viewer-error', event => setStatus(event.detail, true));
  const unsubscribe = bridge.onData?.(receive);
  receive(bridge.data);
  return {
    receive,
    select,
    getState: () => ({ ...state }),
    dispose: () => {
      disposed = true;
      ++loadGeneration;
      unsubscribe?.();
      viewer?.dispose();
      viewer = null;
      meshReady = false;
      records.clear();
      models = [];
      active = null;
      content = null;
    },
  };
}
