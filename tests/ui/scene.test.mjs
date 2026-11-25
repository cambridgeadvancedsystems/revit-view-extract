import test from 'node:test';
import assert from 'node:assert/strict';
import * as THREE from 'three';
import { parseScene, pickElement, disposeObject } from '../../views/src/scene.js';
import { fixture } from './fixture.mjs';

// Node has no browser progress events; GLTFLoader emits them while decoding a data URI.
globalThis.ProgressEvent ??= class extends Event {
  constructor(type, options) {
    super(type);
    Object.assign(this, options);
  }
};

test('real GLTFLoader decodes the embedded mesh and raycasting resolves the engineering ID', async () => {
  const root = await parseScene(fixture().mesh.gltf);
  const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
  camera.position.set(0, 0, 5);
  camera.lookAt(0, 0, 0);
  assert.equal(pickElement(root, camera, 0, 0), '101');
  camera.position.set(4, 0, 5);
  camera.lookAt(4, 0, 0);
  assert.equal(pickElement(root, camera, 0, 0), '202');
  root.children[1].visible = false;
  assert.equal(pickElement(root, camera, 0, 0), null);
  let disposed = 0;
  root.traverse(object => object.geometry?.addEventListener('dispose', () => { disposed += 1; }));
  disposeObject(root);
  assert.ok(disposed >= 1);
});

test('embedded geometry parses and picks with every fetch call forbidden by the sandbox', async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = () => { throw new Error('Network and data-URI fetching forbidden'); };
  try {
    const root = await parseScene(fixture().mesh.gltf);
    const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
    camera.position.set(0, 0, 5);
    camera.lookAt(0, 0, 0);
    assert.equal(pickElement(root, camera, 0, 0), '101');
    disposeObject(root);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('multiple views share one decoded buffer and honour nonzero offsets', async () => {
  const value = fixture().mesh.gltf;
  const positions = Buffer.from(value.buffers[0].uri.split(',')[1], 'base64');
  const bytes = Buffer.alloc(54);
  positions.copy(bytes, 8);
  bytes.writeUInt16LE(0, 48);
  bytes.writeUInt16LE(1, 50);
  bytes.writeUInt16LE(2, 52);
  value.buffers[0] = { byteLength: bytes.length, uri: `data:application/octet-stream;base64,${bytes.toString('base64')}` };
  value.bufferViews = [{ buffer: 0, byteOffset: 8, byteLength: 36 }, { buffer: 0, byteOffset: 48, byteLength: 6 }];
  value.accessors.push({ bufferView: 1, componentType: 5123, count: 3, type: 'SCALAR' });
  value.meshes[0].primitives[0].indices = 1;
  const originalAtob = globalThis.atob;
  let decodes = 0;
  globalThis.atob = input => {
    decodes += 1;
    return originalAtob(input);
  };
  try {
    const root = await parseScene(value);
    const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
    camera.position.set(0, 0, 5);
    camera.lookAt(0, 0, 0);
    assert.equal(pickElement(root, camera, 0, 0), '101');
    assert.equal(decodes, 1);
    disposeObject(root);
  } finally {
    globalThis.atob = originalAtob;
  }
});

test('large decimal-string mesh IDs pick and join engineering properties losslessly', async () => {
  const { buildElementIndex } = await import('../../views/src/data.js');
  const value = fixture();
  const id = '9007199254740993';
  value.mesh.gltf.nodes[0].extras.elementId = id;
  value.engineering.electrical_engineering.panels_and_equipment[0].element_id = id;
  const root = await parseScene(JSON.parse(JSON.stringify(value.mesh.gltf)));
  const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
  camera.position.set(0, 0, 5);
  camera.lookAt(0, 0, 0);
  const picked = pickElement(root, camera, 0, 0);
  assert.equal(picked, id);
  assert.equal(buildElementIndex(value.engineering, value.mesh.gltf).get(picked).voltage_v, 480);
  disposeObject(root);
});

test('selection restores authored emissive values and disposes unique shared resources once', async () => {
  const { prepareMaterials, updateAppearance } = await import('../../views/src/scene.js');
  const root = await parseScene(fixture().mesh.gltf);
  const original = root.children[0].material;
  original.emissive.set('#542819');
  original.emissiveIntensity = 0.25;
  const colour = original.emissive.clone();
  let originalsDisposed = 0;
  original.addEventListener('dispose', () => { originalsDisposed += 1; });
  prepareMaterials(root);
  assert.equal(originalsDisposed, 1);
  assert.notEqual(root.children[0].material, root.children[1].material);
  updateAppearance(root, { selectedId: '101', isolate: true, wireframe: true });
  assert.equal(root.children[1].visible, false);
  assert.equal(root.children[0].material.wireframe, true);
  updateAppearance(root, { selectedId: null, isolate: false, wireframe: false });
  assert.ok(root.children.every(object => object.material.emissive.equals(colour) && object.material.emissiveIntensity === 0.25));
  let geometriesDisposed = 0;
  let materialsDisposed = 0;
  const geometries = new Set(root.children.map(object => object.geometry));
  for (const geometry of geometries) geometry.addEventListener('dispose', () => { geometriesDisposed += 1; });
  for (const object of root.children) object.material.addEventListener('dispose', () => { materialsDisposed += 1; });
  disposeObject(root);
  assert.equal(geometriesDisposed, geometries.size);
  assert.equal(materialsDisposed, 2);
});

test('a truncated embedded buffer fails before constructing geometry', async () => {
  const gltf = fixture().mesh.gltf;
  gltf.buffers[0].uri = `data:application/octet-stream;base64,${Buffer.alloc(4).toString('base64')}`;
  await assert.rejects(parseScene(gltf), /buffer is truncated/);
});
