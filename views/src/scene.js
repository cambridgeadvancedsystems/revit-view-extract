import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { normalizeElementId } from './data.js';
import { assertEmbeddedGltf, decodeBase64 } from './mesh-data.js';

export async function parseScene(gltf) {
  assertEmbeddedGltf(gltf);
  const loader = new GLTFLoader();
  const buffers = new Map();
  // Jaz's CSP forbids even fetch(data:). Decode contained buffers directly.
  loader.register(() => ({
    name: 'LOCAL_EMBEDDED_BUFFERS',
    loadBufferView(index) {
      const view = gltf.bufferViews[index];
      if (!buffers.has(view.buffer)) {
        const uri = gltf.buffers[view.buffer].uri;
        const buffer = decodeBase64(uri.slice(uri.indexOf(',') + 1)).buffer;
        if (buffer.byteLength < gltf.buffers[view.buffer].byteLength) throw new Error('The embedded mesh buffer is truncated. Re-extract the model.');
        buffers.set(view.buffer, buffer);
      }
      const buffer = buffers.get(view.buffer);
      const offset = view.byteOffset ?? 0;
      const length = view.byteLength;
      return Promise.resolve(buffer.slice(offset, offset + length));
    },
  }));
  return (await loader.parseAsync(gltf, '')).scene;
}

export function meshElementId(object) {
  for (let node = object; node; node = node.parent) {
    const id = normalizeElementId(node.userData?.elementId);
    if (id !== null) return id;
  }
  return null;
}

export function pickElement(root, camera, x, y) {
  root.updateMatrixWorld(true);
  camera.updateMatrixWorld(true);
  const raycaster = new THREE.Raycaster();
  raycaster.setFromCamera(new THREE.Vector2(x, y), camera);
  return raycaster.intersectObject(root, true)
    .filter(hit => hit.object.visible && visibleAncestors(hit.object))
    .map(hit => meshElementId(hit.object))
    .find(id => id != null) ?? null;
}

function visibleAncestors(object) {
  for (let node = object.parent; node; node = node.parent) {
    if (!node.visible) return false;
  }
  return true;
}

export function disposeObject(root) {
  const geometries = new Set();
  const materials = new Set();
  root.traverse(object => {
    if (object.geometry) geometries.add(object.geometry);
    for (const material of materialsOf(object)) {
      if (material) materials.add(material);
    }
  });
  for (const geometry of geometries) geometry.dispose();
  for (const material of materials) material.dispose();
}

const originalAppearance = new WeakMap();
const selectedColour = new THREE.Color('#137b8a');
const materialsOf = object => Array.isArray(object.material) ? object.material : [object.material];

export function prepareMaterials(root) {
  const originals = new Set();
  root.traverse(object => {
    if (!object.isMesh) return;
    const copies = materialsOf(object).map(material => {
      originals.add(material);
      const copy = material.clone();
      copy.side = THREE.DoubleSide;
      originalAppearance.set(copy, { colour: copy.emissive?.clone(), intensity: copy.emissiveIntensity });
      return copy;
    });
    object.material = Array.isArray(object.material) ? copies : copies[0];
  });
  for (const material of originals) material.dispose();
}

export function updateAppearance(root, { selectedId, isolate, wireframe }) {
  root.traverse(object => {
    if (!object.isMesh) return;
    const selected = meshElementId(object) === selectedId;
    object.visible = !isolate || selected;
    for (const material of materialsOf(object)) {
      material.wireframe = wireframe;
      if (material.emissive) {
        const original = originalAppearance.get(material);
        material.emissive.copy(selected ? selectedColour : original.colour);
        material.emissiveIntensity = selected ? 0.65 : original.intensity;
      }
    }
  });
}

export class MeshViewer {
  constructor(container, onSelect) {
    if (typeof ResizeObserver !== 'function') throw new Error('This browser cannot size the 3D view. Use a current browser.');
    this.container = container;
    this.disposed = false;
    this.onSelect = onSelect;
    this.scene = new THREE.Scene();
    this.scene.background = new THREE.Color('#edf0f1');
    this.camera = new THREE.PerspectiveCamera(42, 1, 0.01, 100000);
    this.renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    this.renderer.domElement.setAttribute('aria-label', '3D model. Drag to orbit, scroll to zoom, shift-drag to pan. Select elements in the list using the keyboard.');
    this.renderer.domElement.setAttribute('role', 'img');
    this.renderer.domElement.style.touchAction = 'none';
    container.append(this.renderer.domElement);
    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.addEventListener('change', () => this.render());
    this.scene.add(new THREE.HemisphereLight(0xffffff, 0x70818c, 2.5));
    const sun = new THREE.DirectionalLight(0xffffff, 2.8);
    sun.position.set(8, 15, 10);
    this.scene.add(sun);
    this.observer = new ResizeObserver(() => this.resize());
    this.observer.observe(container);
    this.pointerStart = null;
    this.renderer.domElement.addEventListener('pointerdown', event => {
      if (event.button === 0 && event.isPrimary !== false) this.pointerStart = { id: event.pointerId, x: event.clientX, y: event.clientY, moved: false };
    });
    this.renderer.domElement.addEventListener('pointermove', event => {
      const start = this.pointerStart;
      if (start && Math.hypot(event.clientX - start.x, event.clientY - start.y) > 5) start.moved = true;
    });
    this.renderer.domElement.addEventListener('pointercancel', () => { this.pointerStart = null; });
    this.renderer.domElement.addEventListener('pointerup', event => {
      const start = this.pointerStart;
      this.pointerStart = null;
      if (!this.root || !start || start.moved || start.id !== event.pointerId || event.button !== 0) return;
      const rect = this.renderer.domElement.getBoundingClientRect();
      const id = pickElement(this.root, this.camera, (event.clientX - rect.left) / rect.width * 2 - 1, 1 - (event.clientY - rect.top) / rect.height * 2);
      if (id != null) this.onSelect(id);
    });
    this.renderer.domElement.addEventListener('webglcontextlost', event => {
      event.preventDefault();
      if (this.disposed) return;
      container.dispatchEvent(new CustomEvent('viewer-error', { detail: 'The graphics context was lost. Reopen this view to restore 3D rendering.' }));
    });
    this.resize();
  }

  async load(gltf) {
    if (this.disposed) throw new Error('This 3D view has been closed.');
    const generation = (this.generation ?? 0) + 1;
    this.generation = generation;
    const root = await parseScene(gltf);
    if (generation !== this.generation) {
      disposeObject(root);
      return;
    }
    if (this.root) {
      this.scene.remove(this.root);
      disposeObject(this.root);
    }
    this.root = root;
    prepareMaterials(root);
    this.scene.add(root);
    this.fit();
  }

  fit(selectedId = null) {
    if (!this.root) return;
    const box = new THREE.Box3();
    this.root.updateMatrixWorld(true);
    this.root.traverse(object => {
      if (object.isMesh && (!selectedId || meshElementId(object) === selectedId)) box.expandByObject(object);
    });
    if (box.isEmpty()) {
      this.render();
      return;
    }
    const centre = box.getCenter(new THREE.Vector3());
    const size = Math.max(box.getSize(new THREE.Vector3()).length(), 0.01);
    const aspect = Math.max(Math.min(this.camera.aspect, 1), 0.1);
    const distance = size / (2 * Math.tan(THREE.MathUtils.degToRad(this.camera.fov / 2))) / aspect;
    this.camera.near = Math.max(size / 10000, 0.0001);
    this.camera.far = Math.max(size * 100, 100);
    this.camera.position.copy(centre).add(new THREE.Vector3(1, 0.7, 1).normalize().multiplyScalar(distance));
    this.camera.updateProjectionMatrix();
    this.controls.target.copy(centre);
    this.controls.update();
    this.render();
  }

  update({ selectedId, isolate, wireframe }) {
    if (this.root) updateAppearance(this.root, { selectedId, isolate, wireframe });
    this.render();
  }

  resize() {
    if (this.disposed) return;
    const width = Math.max(this.container.clientWidth, 1);
    const height = Math.max(this.container.clientHeight, 1);
    this.renderer.setSize(width, height);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    this.render();
  }

  render() {
    if (!this.disposed) this.renderer.render(this.scene, this.camera);
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.generation = (this.generation ?? 0) + 1;
    this.observer.disconnect();
    this.controls.dispose();
    if (this.root) disposeObject(this.root);
    this.root = null;
    this.scene.clear();
    this.renderer.forceContextLoss();
    this.renderer.dispose();
    this.renderer.domElement.remove();
  }
}
