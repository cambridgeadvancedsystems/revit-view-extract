export function fixture() {
  const positions = new Float32Array([-1, -1, 0, 1, -1, 0, 0, 1, 0]);
  const gltf = {
    asset: { version: '2.0' },
    scene: 0,
    scenes: [{ nodes: [0, 1] }],
    nodes: [
      { mesh: 0, name: 'Test panel', extras: { elementId: '101', ifcType: 'ElectricalEquipment' } },
      { mesh: 0, translation: [4, 0, 0], name: 'Test fan', extras: { elementId: '202', ifcType: 'MechanicalEquipment' } },
    ],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 }, material: 0 }] }],
    materials: [{ pbrMetallicRoughness: { baseColorFactor: [0.4, 0.6, 0.7, 1], metallicFactor: 0, roughnessFactor: 1 } }],
    buffers: [{ byteLength: positions.byteLength, uri: `data:application/octet-stream;base64,${Buffer.from(positions.buffer).toString('base64')}` }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: positions.byteLength }],
    accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [-1, -1, 0], max: [1, 1, 0] }],
  };
  return {
    schema_version: 1,
    source: { name: 'Synthetic test model' },
    engineering: {
      electrical_engineering: { panels_and_equipment: [{ element_id: 101, panel_name: 'Test panel', voltage_v: 480, category: 'ElectricalEquipment', metadata: { manufacturer: '<script>unsafe</script>' } }] },
      hvac_engineering: { mechanical_equipment: [{ element_id: 202, name: 'Test fan', category: 'MechanicalEquipment', airflow_cfm: 500 }] },
    },
    mesh: { gltf },
  };
}
