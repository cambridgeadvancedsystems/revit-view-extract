import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeModels, buildElementIndex, filterElements, filterProperties } from '../../views/src/data.js';
import { assertEmbeddedGltf, decodeMesh } from '../../views/src/mesh-data.js';
import { fixture } from './fixture.mjs';

test('thread and collection data produce the same engineering index', () => {
  const value = fixture();
  const [thread] = normalizeModels(value);
  const [page] = normalizeModels({ items: [{ folder: '/synthetic', data: value, threads: [{ id: 'thread-1' }] }] });
  assert.deepEqual(page.engineering, thread.engineering);
  assert.equal(page.threads[0].id, 'thread-1');
  assert.equal(buildElementIndex(page.engineering, page.gltf).size, 2);
  assert.deepEqual(normalizeModels({ schema_version: 99 }), []);
  assert.deepEqual(normalizeModels(null), []);
});

test('mesh element IDs join numeric engineering IDs and preserve actual values', () => {
  const value = fixture();
  const records = buildElementIndex(value.engineering, value.mesh.gltf);
  assert.equal(records.get('101').voltage_v, 480);
  assert.equal(records.get('101').has_mesh, true);
  assert.equal(filterElements(records, '500')[0].element_id, '202');
  assert.equal(filterElements(records, '', 'ElectricalEquipment').length, 1);
  assert.equal(filterProperties(records.get('101'), 'manufacturer')[0][1], '<script>unsafe</script>');
  assert.equal(filterProperties(records.get('101'), '480')[0][0], 'voltage_v');
});

test('viewer rejects every external asset before GLTFLoader can make a request', () => {
  const gltf = fixture().mesh.gltf;
  assert.doesNotThrow(() => assertEmbeddedGltf(gltf));
  for (const uri of ['file:///private/model.bin', 'https://example.org/model.bin', '../model.bin', 'data:text/html;base64,AA==']) {
    assert.throws(() => assertEmbeddedGltf({ ...gltf, buffers: [{ uri }] }), /embedded/);
  }
  assert.throws(() => assertEmbeddedGltf({ ...gltf, images: [{ uri: 'https://example.org/texture.png' }] }), /Textured/);
  assert.throws(() => assertEmbeddedGltf({ ...gltf, extensionsRequired: ['KHR_draco_mesh_compression'] }), /extended/);
});

test('compressed model transport roundtrips exactly and rejects malformed data', async () => {
  const { gzipSync } = await import('node:zlib');
  const gltf = fixture().mesh.gltf;
  const compressed = gzipSync(JSON.stringify(gltf)).toString('base64');
  assert.deepEqual(await decodeMesh({ gltfGzip: compressed }), gltf);
  await assert.rejects(decodeMesh({ gltfGzip: 'not-base64!' }), /compressed mesh is invalid/);
  const badJson = gzipSync('{broken json').toString('base64');
  await assert.rejects(decodeMesh({ gltfGzip: badJson }), /compressed mesh is invalid/);
});

test('the full declared element index joins saved parameters for non-equipment elements', () => {
  const records = buildElementIndex({
    element_index: [{ element_id: 303, class_name: 'Wall', verified_record: true }],
    parameters_by_element: { '303': [{ parameter_id: -100, name: 'Fire rating', value: '60 min' }] },
  });
  assert.equal(records.get('303').class_name, 'Wall');
  assert.ok(filterProperties(records.get('303'), 'Fire rating').some(([name, value]) => name.endsWith('.value') && value === '60 min'));
});

test('searching a parameter name returns its raw and converted values with units', () => {
  const record = { parameters: [{ name: 'Electrical voltage', level: 'instance', value: 1722.2256, si_value: 160, si_unit: 'V' }] };
  const rows = filterProperties(record, 'voltage');
  assert.ok(rows.some(([name, value]) => name.endsWith('.value') && value === '1722.2256'));
  assert.ok(rows.some(([name, value]) => name.endsWith('.si_value') && value === '160'));
  assert.ok(rows.some(([name, value]) => name.endsWith('.si_unit') && value === 'V'));
});

test('IDs above the JavaScript integer range remain lossless through JSON and the index', () => {
  const value = fixture();
  const id = '9007199254740993';
  value.engineering.electrical_engineering.panels_and_equipment[0].element_id = id;
  value.mesh.gltf.nodes[0].extras.elementId = id;
  const parsed = JSON.parse(JSON.stringify(value));
  const records = buildElementIndex(parsed.engineering, parsed.mesh.gltf);
  assert.equal(records.get(id).voltage_v, 480);
  assert.equal(records.get(id).has_mesh, true);
  assert.throws(() => buildElementIndex({ element_index: [{ element_id: Number(id) }] }), /decimal strings/);
});

test('malformed collection items are skipped and valid model keys survive collection reordering', () => {
  const first = { folder: '/first', data: fixture() };
  const second = { folder: '/second', data: fixture() };
  const before = normalizeModels({ items: [null, first, second] });
  const after = normalizeModels({ items: [second, first] });
  assert.equal(before.length, 2);
  assert.equal(before[0].key, after[1].key);
  assert.deepEqual(normalizeModels({ unrelated: true }), []);
  assert.doesNotThrow(() => filterProperties({ parameters: [null, 7, { name: 'Valid', value: 5 }] }, 'Valid'));
});

test('search caches are replaced with new model records when values change', () => {
  const first = buildElementIndex({ element_index: [{ element_id: '1', name: 'Before' }] });
  const second = buildElementIndex({ element_index: [{ element_id: '1', name: 'After' }] });
  assert.equal(filterElements(first, 'Before').length, 1);
  assert.equal(filterElements(second, 'Before').length, 0);
  assert.equal(filterElements(second, 'After').length, 1);
});

test('array-shaped asset metadata cannot bypass external-asset rejection', () => {
  const gltf = fixture().mesh.gltf;
  assert.throws(() => assertEmbeddedGltf({ ...gltf, images: { length: 0, 0: { uri: 'https://example.org/texture.png' } } }), /must be an array/);
  assert.throws(() => assertEmbeddedGltf({ ...gltf, buffers: { length: 0 } }), /must be an array/);
  assert.throws(() => assertEmbeddedGltf({ ...gltf, bufferViews: [{ buffer: 0, byteOffset: 20, byteLength: 1000 }] }), /buffer range/);
});
