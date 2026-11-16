import http from 'node:http';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const port = Number(process.env.PORT || 4177);
const mime = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.wasm': 'application/wasm',
  '.png': 'image/png',
  '.webp': 'image/webp',
  '.pdb': 'text/plain',
  '.cif': 'text/plain',
  '.ttf': 'font/ttf',
};

http.createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    const file = path.resolve(root, '.' + pathname + (pathname.endsWith('/') ? 'index.html' : ''));
    const relative = path.relative(root, file);
    const allowedRoot = /^(assets|js|docs|examples)\//.test(relative);
    const allowedPage = /^[\w-]+\.html$/.test(relative);
    if (relative.startsWith('..') || path.isAbsolute(relative) || relative.split(path.sep).some(part => part.startsWith('.')) || (!allowedRoot && !allowedPage)) {
      response.writeHead(403).end('Forbidden');
      return;
    }
    const data = await readFile(file);
    response.writeHead(200, {
      'Content-Type': mime[path.extname(file)] || 'application/octet-stream',
      'Cache-Control': 'no-store',
    }).end(data);
  } catch (error) {
    response.writeHead(error.code === 'ENOENT' ? 404 : 400).end('Cannot read this file');
  }
}).listen(port, '127.0.0.1', () => {
  console.log(`Patinae preview: http://127.0.0.1:${port}`);
});
