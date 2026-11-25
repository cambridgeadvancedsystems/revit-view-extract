import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { fixture } from '../tests/ui/fixture.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const port = Number(process.env.REVIT_PREVIEW_PORT ?? 4178);
createServer(async (request, response) => {
  const url = new URL(request.url, 'http://localhost');
  const view = url.pathname === '/inspector' ? 'inspector' : 'viewer';
  let data = url.searchParams.has('page')
    ? { items: [{ folder: '/synthetic', data: fixture(), threads: [] }] }
    : fixture();
  try {
    const model = url.searchParams.get('model');
    if (process.env.REVIT_PREVIEW_DATA_DIR && ['electrical', 'hvac', 'architectural'].includes(model)) {
      const filename = `${model}.json`;
      data = JSON.parse(await readFile(path.resolve(process.env.REVIT_PREVIEW_DATA_DIR, filename), 'utf8'));
    }
    const html = await readFile(path.join(root, `views/${view}.html`), 'utf8');
    const safeData = JSON.stringify(data).replaceAll('<', '\\u003c');
    let importScript = '';
    if (url.searchParams.has('importTest') && data.kind === 'revit-view-descriptor') {
      if (path.basename(data.model_file) !== data.model_file) throw new Error('Expected a plain model filename');
      const text = await readFile(path.resolve(process.env.REVIT_PREVIEW_DATA_DIR, data.model_file), 'utf8');
      const literal = JSON.stringify(text).replaceAll('<', '\\u003c');
      importScript = `window.__localImportText=${literal};`;
    }
    const headers = { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' };
    if (url.searchParams.has('csp')) headers['content-security-policy'] = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; connect-src 'none'";
    response.writeHead(200, headers);
    response.end(html.replace('</head>', () => `<script>window.data=${safeData};${importScript}</script></head>`));
  } catch (error) {
    response.writeHead(500, { 'content-type': 'text/plain' });
    response.end(error.message);
  }
}).listen(port, '127.0.0.1', () => console.log(`Synthetic preview: http://127.0.0.1:${port}`));
