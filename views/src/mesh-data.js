const array = (gltf, name) => {
  if (gltf[name] != null && !Array.isArray(gltf[name])) throw new Error(`The mesh ${name} field must be an array. Re-extract the model.`);
  return gltf[name] ?? [];
};

export function assertEmbeddedGltf(gltf) {
  if (!gltf || typeof gltf !== 'object' || gltf.asset?.version !== '2.0') throw new Error('Expected a glTF 2.0 mesh. Re-extract the model.');
  if (array(gltf, 'extensionsRequired').length) throw new Error('Compressed or extended glTF is not supported. Re-extract the model.');
  if (array(gltf, 'images').length || array(gltf, 'textures').length) throw new Error('Textured glTF is not supported in this local viewer. Re-extract the model.');
  const buffers = array(gltf, 'buffers');
  for (const buffer of buffers) {
    if (!/^data:application\/(?:octet-stream|gltf-buffer);base64,[A-Za-z0-9+/=\r\n]*$/.test(buffer?.uri ?? '')) throw new Error('Mesh buffers must be embedded. Re-extract the model.');
    if (!Number.isSafeInteger(buffer.byteLength) || buffer.byteLength < 0) throw new Error('The mesh buffer length is invalid. Re-extract the model.');
  }
  for (const view of array(gltf, 'bufferViews')) {
    const offset = view?.byteOffset ?? 0;
    if (!Number.isSafeInteger(view?.buffer) || !buffers[view.buffer] || !Number.isSafeInteger(offset) || !Number.isSafeInteger(view.byteLength) || offset < 0 || view.byteLength < 0 || offset + view.byteLength > buffers[view.buffer].byteLength) {
      throw new Error('The embedded mesh contains an invalid buffer range. Re-extract the model.');
    }
  }
  for (const node of array(gltf, 'nodes')) {
    if (!node || typeof node !== 'object' || Array.isArray(node)) throw new Error('The mesh contains an invalid node. Re-extract the model.');
  }
}

export async function decodeMesh(model) {
  if (model.gltf) {
    assertEmbeddedGltf(model.gltf);
    return model.gltf;
  }
  if (typeof model.gltfGzip !== 'string') throw new Error('No saved mesh found');
  if (typeof DecompressionStream !== 'function') throw new Error('This browser cannot decompress the model. Use a current browser or open an uncompressed export');
  try {
    const bytes = decodeBase64(model.gltfGzip);
    const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream('gzip'));
    const gltf = await new Response(stream).json();
    assertEmbeddedGltf(gltf);
    return gltf;
  } catch {
    throw new Error('The compressed mesh is invalid. Re-extract the model');
  }
}

export function decodeBase64(value) {
  const decoded = atob(value);
  const bytes = new Uint8Array(decoded.length);
  for (let index = 0; index < decoded.length; index += 1) bytes[index] = decoded.charCodeAt(index);
  return bytes;
}
