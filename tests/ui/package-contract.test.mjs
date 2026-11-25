import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { Script } from 'node:vm';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const json = async name => JSON.parse(await readFile(path.join(root, name), 'utf8'));

test('portable and compatibility manifests retain thread views and icon without a sidebar page', async () => {
  const primary = await json('plugin.json');
  const compatibility = await json('.claude-plugin/plugin.json');
  const marketplace = await json('.agents/plugins/marketplace.json');
  const [entry] = marketplace.plugins;
  const jaz = primary.extensions['chat.jaz'];
  assert.equal(primary.$schema, 'https://agent-plugins.org/schemas/1.0.0/plugin.schema.json');
  assert.equal(primary.name, 'revit-view-extract');
  assert.equal(primary.name, compatibility.name);
  assert.equal(primary.version, compatibility.version);
  assert.equal(jaz.displayName, 'Revit Viewer');
  assert.equal(jaz.displayName, compatibility.displayName);
  assert.equal(jaz.icon, compatibility.logo);
  assert.equal(compatibility.category, 'engineering');
  assert.deepEqual(Object.keys(jaz).sort(), ['displayName', 'icon']);
  assert.equal(Object.hasOwn(primary, 'page'), false);
  assert.equal(Object.hasOwn(compatibility, 'page'), false);
  assert.match(jaz.icon, /^\.\/assets\/[a-z.-]+\.svg$/);
  assert.ok((await stat(path.join(root, jaz.icon))).size < 512 * 1024);
  assert.ok((await readFile(path.join(root, jaz.icon), 'utf8')).includes('currentColor'));
  for (const name of ['viewer', 'inspector']) {
    assert.ok((await stat(path.join(root, `views/${name}.html`))).isFile());
  }
  assert.ok((await stat(path.join(root, 'skills/revit-view-extract/SKILL.md'))).isFile());
  assert.equal(marketplace.name, primary.name);
  assert.equal(marketplace.plugins.length, 1);
  assert.equal(entry.name, primary.name);
  assert.equal(entry.displayName, jaz.displayName);
  assert.equal(entry.source, './');
});

test('both shipped HTML bundles compile and have no package-relative runtime dependencies', async () => {
  for (const name of ['viewer', 'inspector']) {
    const html = await readFile(path.join(root, `views/${name}.html`), 'utf8');
    const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
    assert.equal(scripts.length, 1);
    assert.doesNotThrow(() => new Script(scripts[0][1]));
    assert.equal(html.includes('/* SCRIPT */'), false);
    assert.equal(html.includes('/* STYLE */'), false);
    assert.doesNotMatch(html, /<script[^>]*\bsrc=|<link[^>]*\bhref=/i);
    assert.ok(html.includes(`data-view="${name}"`));
    assert.ok(html.includes('<title>Revit Viewer</title>'));
    assert.ok(html.includes('<h1 id="source-name">Revit Viewer</h1>'));
  }
});
