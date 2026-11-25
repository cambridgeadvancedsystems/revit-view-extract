const isObject = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const engineeringKeys = ['element_index', 'parameters_by_element', 'electrical_engineering', 'hvac_engineering', 'levels', 'sheets', 'schedules', 'text_notes', 'elem_table_total'];
const searchText = new WeakMap();

/** IDs from current exports are decimal strings; safe legacy integers remain supported. */
export function normalizeElementId(value) {
  if (typeof value === 'number' && !Number.isSafeInteger(value)) {
    throw new Error('Element IDs outside the safe integer range must be decimal strings. Re-extract this model.');
  }
  return (typeof value === 'string' || typeof value === 'number') && /^-?\d+$/.test(String(value)) ? String(value) : null;
}

export function restoreState(value) {
  const saved = isObject(value) ? value : {};
  const string = key => typeof saved[key] === 'string' ? saved[key] : '';
  let selectedId = null;
  try { selectedId = normalizeElementId(saved.selectedId); } catch { /* Ignore an invalid remembered selection. */ }
  return {
    selectedId,
    query: string('query'),
    category: string('category'),
    propertyQuery: string('propertyQuery'),
    modelKey: string('modelKey'),
    isolate: false,
    wireframe: saved.wireframe === true,
  };
}

/** Normalise the Jaz page catalogue and thread data to the same model contract. */
export function normalizeModels(payload) {
  const items = Array.isArray(payload?.items) ? payload.items : [{ data: payload }];
  return items.flatMap((item, index) => {
    if (!isObject(item) || !isObject(item.data)) return [];
    const data = item.data;
    if (data.schema_version != null && data.schema_version !== 1) return [];
    const descriptor = data.kind === 'revit-view-descriptor';
    if (descriptor && (typeof data.document_id !== 'string' || !data.document_id || typeof data.model_file !== 'string' || !/^[^/\\\u0000]+\.json$/i.test(data.model_file))) return [];
    if (!descriptor && !isObject(data.engineering) && !isObject(data.mesh) && !engineeringKeys.some(key => key in data)) return [];
    if (data.engineering != null && !isObject(data.engineering)) return [];
    if (data.mesh != null && !isObject(data.mesh)) return [];
    const name = typeof data.source?.name === 'string' ? data.source.name : `Model ${index + 1}`;
    const modelFile = descriptor ? data.model_file : null;
    return [{
      key: JSON.stringify([item.folder ?? '', modelFile ?? name]),
      name,
      descriptor,
      documentId: data.document_id ?? null,
      modelFile,
      summary: isObject(data.summary) ? data.summary : {},
      engineering: descriptor ? {} : data.engineering ?? (data.mesh ? {} : data),
      gltf: data.mesh?.gltf ?? null,
      gltfGzip: data.mesh?.gltf_gzip_base64 ?? null,
      metrics: data.mesh?.metrics ?? null,
      threads: Array.isArray(item.threads) ? item.threads.filter(thread => isObject(thread) && typeof thread.id === 'string') : [],
    }];
  });
}

export function buildElementIndex(engineering, gltf) {
  const records = new Map();
  const pending = [[engineering, '']];
  while (pending.length) {
    const [value, section] = pending.pop();
    if (!value || typeof value !== 'object') continue;
    if (Array.isArray(value)) {
      for (let index = value.length - 1; index >= 0; index -= 1) pending.push([value[index], section]);
      continue;
    }
    const id = normalizeElementId(value.element_id ?? value.elementId);
    if (id !== null) {
      const previous = records.get(id);
      records.set(id, { ...previous, ...value, element_id: id, section: previous?.section ?? section });
    }
    const entries = Object.entries(value);
    for (let index = entries.length - 1; index >= 0; index -= 1) {
      const [key, child] = entries[index];
      if (key !== 'parameters' && key !== 'parameters_by_element' && child && typeof child === 'object') pending.push([child, section || key]);
    }
  }
  for (const [id, parameters] of Object.entries(engineering?.parameters_by_element ?? {})) {
    const key = normalizeElementId(id);
    if (key !== null) records.set(key, { ...records.get(key), element_id: key, parameters });
  }
  for (const node of Array.isArray(gltf?.nodes) ? gltf.nodes : []) {
    if (!isObject(node) || node.mesh == null) continue;
    const key = normalizeElementId(node.extras?.elementId);
    if (key === null) continue;
    const previous = records.get(key);
    records.set(key, {
      name: node.name ?? `Element ${key}`,
      section: 'saved_meshes',
      ...previous,
      element_id: key,
      has_mesh: true,
      mesh_type: node.extras?.ifcType ?? '',
    });
  }
  return records;
}

export function elementLabel(record) {
  return String(record.panel_name || record.name || record.load_name || record.family || `Element ${record.element_id}`);
}

export function elementCategory(record) {
  return String(record.category || record.mesh_type || record.class_name || record.section || 'Other');
}

export function flattenProperties(value, prefix = '') {
  const result = [];
  const pending = [[prefix, value]];
  while (pending.length) {
    const [path, current] = pending.pop();
    if (!current || typeof current !== 'object') {
      result.push([path, current == null ? '—' : String(current)]);
      continue;
    }
    const parameters = Array.isArray(current) && (path === 'parameters' || path.endsWith('.parameters'));
    const entries = Object.entries(current);
    for (let index = entries.length - 1; index >= 0; index -= 1) {
      const [key, child] = entries[index];
      if (parameters && isObject(child)) {
        const label = child.name || `Parameter ${child.parameter_id ?? key}`;
        const scope = child.level ? ` [${child.level}]` : '';
        const fields = Object.entries(child).filter(([field]) => field !== 'name');
        for (let field = fields.length - 1; field >= 0; field -= 1) pending.push([`${path}.${label}${scope}.${fields[field][0]}`, fields[field][1]]);
      } else {
        pending.push([path ? `${path}.${key}` : key, child]);
      }
    }
  }
  return result;
}

export function filterElements(records, query = '', category = '') {
  const needle = query.trim().toLocaleLowerCase();
  return [...records.values()].filter(record => {
    if (category && elementCategory(record) !== category) return false;
    if (!needle) return true;
    if (!searchText.has(record)) searchText.set(record, flattenProperties(record).map(([key, value]) => `${key} ${value}`.toLocaleLowerCase()).join('\n'));
    return searchText.get(record).includes(needle);
  });
}

export function filterProperties(record, query = '') {
  const needle = query.trim().toLocaleLowerCase();
  return flattenProperties(record).filter(([key, value]) => `${key} ${value}`.toLocaleLowerCase().includes(needle));
}
