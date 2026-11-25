import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { parseHTML } from 'linkedom';
import { gzipSync } from 'node:zlib';
import { mountApp } from '../../views/src/app.js';
import { fixture } from './fixture.mjs';

const template = await readFile(new URL('../../views/src/template.html', import.meta.url), 'utf8');
function harness(data, view = 'viewer', viewState = undefined) {
  const { document, window } = parseHTML(template.replace('VIEW_NAME', view));
  // LinkeDOM exposes the select getter only. Mirror the browser value setter.
  Object.defineProperty(window.HTMLSelectElement.prototype, 'value', {
    get() { return [...this.options].find(option => option.selected)?.value ?? ''; },
    set(value) { for (const option of this.options) option.selected = option.value === value; },
    configurable: true,
  });
  let receive;
  const saved = [];
  const calls = [];
  let choose;
  const viewer = {
    load: async gltf => { calls.push(['load', gltf]); },
    update: state => { calls.push(['update', { ...state }]); },
    fit: id => { calls.push(['fit', id]); },
    dispose: () => { calls.push(['dispose']); },
  };
  const bridge = { data, viewState, onData: callback => { receive = callback; }, setViewState: state => saved.push(state), openView: (...args) => calls.push(['openView', ...args]), openLink: (...args) => calls.push(['openLink', ...args]) };
  const app = mountApp(document, bridge, (_container, onSelect) => { choose = onSelect; return viewer; });
  const fire = (id, type, value) => {
    const node = document.getElementById(id);
    if (value !== undefined) node.value = value;
    node.dispatchEvent(new window.Event(type, { bubbles: true }));
  };
  return { document, app, fire, calls, saved, receive: value => receive(value), pick: id => choose(id) };
}

test('production UI connects a picked mesh to properties, searches and isolating', async () => {
  const h = harness(fixture());
  await new Promise(setImmediate);
  h.pick('101');
  assert.equal(h.document.getElementById('selected-name').textContent, 'Test panel');
  assert.ok(h.document.getElementById('properties').textContent.includes('480'));
  assert.equal(h.document.querySelector('#properties script'), null);
  assert.ok(h.document.getElementById('properties').textContent.includes('<script>unsafe</script>'));
  h.fire('property-search', 'input', 'voltage');
  assert.equal(h.document.querySelectorAll('#properties tr').length, 1);
  h.fire('isolate', 'click');
  assert.equal(h.app.getState().isolate, true);
  assert.equal(h.calls.filter(call => call[0] === 'update').at(-1)[1].selectedId, '101');
  h.fire('focus', 'click');
  assert.deepEqual(h.calls.at(-1), ['fit', '101']);
  h.fire('show-all', 'click');
  assert.equal(h.app.getState().isolate, false);
  h.fire('element-search', 'input', '500');
  assert.equal(h.document.querySelectorAll('.element-row').length, 1);
  assert.equal(h.document.querySelector('.element-row').getAttribute('data-element-id'), '202');
  assert.ok(h.saved.length > 0);
  h.app.dispose();
});

test('live bridge updates drop removed selections and replace model content', async () => {
  const h = harness(fixture());
  await Promise.resolve();
  h.pick('101');
  const next = fixture();
  next.engineering.electrical_engineering.panels_and_equipment = [];
  next.mesh.gltf.nodes = next.mesh.gltf.nodes.slice(1);
  h.receive(next);
  await Promise.resolve();
  assert.equal(h.app.getState().selectedId, null);
  assert.equal(h.document.querySelectorAll('.element-row').length, 1);
  assert.ok(h.calls.some(call => call[0] === 'dispose'));
  h.app.dispose();
});

test('collection models open their associated thread; inspector works without WebGL', async () => {
  const data = fixture();
  const h = harness({ items: [{ folder: '/one', data, threads: [{ id: 'synthetic-id' }] }, { folder: '/two', data: { ...data, source: { name: 'Second model' } }, threads: [] }] }, 'inspector');
  await Promise.resolve();
  assert.equal(h.document.getElementById('model').hidden, false);
  assert.equal(h.document.getElementById('other-view').hidden, true);
  h.fire('open-thread', 'click');
  assert.deepEqual(h.calls.at(-1), ['openLink', '/sessions/synthetic-id']);
  assert.equal(h.calls.some(call => call[0] === 'load'), false);
  h.document.querySelector('.element-row').click();
  assert.equal(h.document.getElementById('selected-name').textContent, 'Test panel');
  assert.equal(h.document.getElementById('focus').disabled, true);
  h.app.dispose();
});

test('empty and engineering-only exports retain working property inspection', () => {
  const h = harness(null, 'inspector');
  assert.equal(h.document.querySelectorAll('.element-row').length, 0);
  h.receive(fixture().engineering);
  h.document.querySelector('.element-row').click();
  assert.ok(h.document.getElementById('properties').textContent.includes('480'));
  h.fire('property-search', 'input', 'missing-value');
  assert.equal(h.document.getElementById('property-empty').hidden, false);
  h.app.dispose();
});

test('failed WebGL loading presents a recoverable error with searchable properties', async () => {
  const { document } = parseHTML(template.replace('VIEW_NAME', 'viewer'));
  let receive;
  const app = mountApp(document, { data: fixture(), onData: callback => { receive = callback; } }, () => { throw new Error('WebGL disabled'); });
  await Promise.resolve();
  assert.match(document.getElementById('mesh-status').textContent, /3D unavailable: WebGL disabled/);
  document.querySelector('.element-row').click();
  assert.ok(document.getElementById('properties').textContent.includes('480'));
  receive(null);
  assert.equal(document.querySelectorAll('.element-row').length, 0);
  app.dispose();
});

test('an older pending mesh load cannot replace newer data or enable stale controls', async () => {
  const { document } = parseHTML(template.replace('VIEW_NAME', 'viewer'));
  const resolvers = [];
  const views = [];
  let receive;
  const app = mountApp(document, { data: fixture(), onData: callback => { receive = callback; } }, () => {
    const view = {
      disposed: false,
      load: () => new Promise(resolve => { resolvers.push(resolve); }),
      update: () => {},
      dispose: () => { view.disposed = true; },
    };
    views.push(view);
    return view;
  });
  await Promise.resolve();
  const next = fixture();
  next.source.name = 'Newer model';
  receive(next);
  await Promise.resolve();
  resolvers[1]();
  await Promise.resolve();
  assert.match(document.getElementById('source-name').textContent, /Newer model/);
  resolvers[0]();
  await Promise.resolve();
  assert.match(document.getElementById('source-name').textContent, /Newer model/);
  assert.equal(views[0].disposed, true);
  assert.equal(views[1].disposed, false);
  app.dispose();
});

test('compact descriptor uses the production File API input and invalidates old imported data', async () => {
  const model = { ...fixture(), document_id: 'export-1' };
  const descriptor = { schema_version: 1, kind: 'revit-view-descriptor', document_id: 'export-1', source: model.source, model_file: 'model.model.json', summary: { elem_table_total: 2 } };
  assert.ok(Buffer.byteLength(JSON.stringify(descriptor)) < 10_000);
  const h = harness(descriptor);
  assert.equal(h.document.querySelectorAll('.element-row').length, 0);
  assert.equal(h.document.getElementById('choose-json').textContent, 'Open extracted model');
  assert.match(h.document.getElementById('mesh-status').textContent, /model.model.json/);
  const input = h.document.getElementById('json-file');
  Object.defineProperty(input, 'files', { configurable: true, value: [new File([JSON.stringify(model)], 'model.model.json', { type: 'application/json' })] });
  h.fire('json-file', 'change');
  await new Promise(setImmediate);
  assert.equal(h.document.querySelectorAll('.element-row').length, 2);
  h.pick('101');
  assert.ok(h.document.getElementById('properties').textContent.includes('480'));
  h.receive({ ...descriptor, document_id: 'export-2' });
  assert.equal(h.document.querySelectorAll('.element-row').length, 0);
  assert.match(h.document.getElementById('mesh-status').textContent, /Model updated/);
  h.fire('json-file', 'change');
  await new Promise(setImmediate);
  assert.match(h.document.getElementById('mesh-status').textContent, /does not match the current export/);
  assert.equal(h.document.querySelectorAll('.element-row').length, 0);
  h.app.dispose();
});

function descriptor(model = { ...fixture(), document_id: 'bound-export' }) {
  return { schema_version: 1, kind: 'revit-view-descriptor', document_id: model.document_id, source: model.source, model_file: 'model.model.json', summary: { elem_table_total: 2 } };
}

function chooseFile(h, file) {
  Object.defineProperty(h.document.getElementById('json-file'), 'files', { configurable: true, value: [file] });
  h.fire('json-file', 'change');
}

const modelFile = model => new File([JSON.stringify(model)], 'model.model.json', { type: 'application/json' });

test('every file replacement remains bound to the original descriptor export', async () => {
  const model = { ...fixture(), document_id: 'bound-export' };
  const h = harness(descriptor(model));
  chooseFile(h, modelFile(model));
  await new Promise(setImmediate);
  assert.equal(h.document.querySelectorAll('.element-row').length, 2);
  assert.equal(h.document.getElementById('choose-json').textContent, 'Open extracted model');
  chooseFile(h, modelFile({ ...model, document_id: 'different-export' }));
  await new Promise(setImmediate);
  assert.equal(h.document.querySelectorAll('.element-row').length, 0);
  assert.match(h.document.getElementById('mesh-status').textContent, /does not match/);
  h.app.dispose();
});

test('a late failed file read cannot overwrite a newer descriptor status', async () => {
  const h = harness(descriptor());
  let fail;
  const old = modelFile(fixture());
  Object.defineProperty(old, 'text', { value: () => new Promise((_resolve, reject) => { fail = reject; }) });
  chooseFile(h, old);
  h.receive({ ...descriptor(), document_id: 'new-export', model_file: 'new.model.json' });
  fail(new Error('Obsolete read failed'));
  await new Promise(setImmediate);
  assert.match(h.document.getElementById('mesh-status').textContent, /new.model.json/);
  assert.doesNotMatch(h.document.getElementById('mesh-status').textContent, /Obsolete/);
  h.app.dispose();
});

test('the latest file selection wins when an older read finishes first', async () => {
  const h = harness(descriptor());
  const completions = [];
  for (const label of ['Old panel', 'New panel']) {
    const value = { ...fixture(), document_id: 'bound-export' };
    value.engineering.electrical_engineering.panels_and_equipment[0].panel_name = label;
    const file = modelFile(value);
    Object.defineProperty(file, 'text', { value: () => new Promise(resolve => { completions.push(() => resolve(JSON.stringify(value))); }) });
    chooseFile(h, file);
  }
  completions[0]();
  await new Promise(setImmediate);
  assert.equal(h.document.querySelectorAll('.element-row').length, 0);
  completions[1]();
  await new Promise(setImmediate);
  h.pick('101');
  assert.equal(h.document.getElementById('selected-name').textContent, 'New panel');
  h.app.dispose();
});

test('selection preserves the keyboard-focused element button', async () => {
  const h = harness(fixture());
  await new Promise(setImmediate);
  const row = h.document.querySelector('.element-row');
  row.click();
  assert.equal(h.document.querySelector('.element-row'), row);
  assert.equal(row.getAttribute('aria-pressed'), 'true');
  h.app.dispose();
});

test('invalid saved preferences do not crash the model and unknown state is discarded', async () => {
  const h = harness(fixture(), 'viewer', { query: null, category: [], propertyQuery: 42, selectedId: 2 ** 60, wireframe: 'yes', unrelated: { large: true } });
  await new Promise(setImmediate);
  assert.equal(h.document.querySelectorAll('.element-row').length, 2);
  assert.equal(h.app.getState().query, '');
  assert.equal(h.app.getState().selectedId, null);
  assert.equal(h.app.getState().wireframe, false);
  assert.equal('unrelated' in h.app.getState(), false);
  h.app.dispose();
});

test('the inspector keeps its error/status and model chooser outside hidden 3D controls', () => {
  const h = harness(descriptor(), 'inspector');
  assert.equal(h.document.getElementById('mesh-status').closest('.stage'), null);
  assert.equal(h.document.getElementById('model').closest('.model-tools'), null);
  h.app.dispose();
});

test('disposal unsubscribes from live host data and prevents reactivation', async () => {
  const { document } = parseHTML(template.replace('VIEW_NAME', 'inspector'));
  let receive;
  let unsubscribed = false;
  const app = mountApp(document, { data: fixture(), onData: callback => {
    receive = callback;
    return () => { unsubscribed = true; };
  } });
  app.dispose();
  receive(null);
  await new Promise(setImmediate);
  assert.equal(unsubscribed, true);
  assert.equal(document.getElementById('source-name').textContent, 'Synthetic test model');
});

test('the inspector decodes compressed mesh metadata without WebGL and restores mesh-only selection', async () => {
  const value = fixture();
  value.engineering.electrical_engineering.panels_and_equipment = [];
  value.mesh = { gltf_gzip_base64: gzipSync(JSON.stringify(value.mesh.gltf)).toString('base64') };
  const h = harness(value, 'inspector', { selectedId: '101' });
  for (let attempt = 0; attempt < 100 && !h.document.getElementById('mesh-status').textContent.includes('saved mesh properties'); attempt += 1) {
    await new Promise(resolve => setTimeout(resolve, 5));
  }
  assert.equal(h.document.querySelectorAll('.element-row').length, 2);
  assert.equal(h.app.getState().selectedId, '101');
  assert.equal(h.document.getElementById('selected-name').textContent, 'Test panel');
  assert.match(h.document.getElementById('properties').textContent, /saved_meshes/);
  assert.equal(h.calls.some(call => call[0] === 'load'), false);
  assert.equal(h.document.getElementById('focus').disabled, true);
  h.app.dispose();
});
