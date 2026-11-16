// A static file server that behaves like GitHub Pages for this project:
// the build is served under the base path, a directory serves its
// index.html, a directory URL without a trailing slash redirects, and
// anything missing gets 404.html with status 404.
//
//   node tests/serve.mjs <dir> <port> [base]
import { createReadStream, existsSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { extname, join, normalize, resolve } from 'node:path';

const [dir = 'dist', port = '4321', base = '/deckpilot'] = process.argv.slice(2);
const root = resolve(dir);
const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.xml': 'application/xml',
  '.txt': 'text/plain; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.woff2': 'font/woff2',
};

createServer((request, response) => {
  const url = new URL(request.url ?? '/', 'http://localhost');
  const send = (status, file) => {
    response.writeHead(status, { 'Content-Type': TYPES[extname(file)] ?? 'application/octet-stream' });
    createReadStream(file).pipe(response);
  };
  const notFound = () => {
    const page = join(root, '404.html');
    if (existsSync(page)) return send(404, page);
    response.writeHead(404).end('not found');
  };
  if (url.pathname !== base && !url.pathname.startsWith(`${base}/`)) return notFound();
  const relative = normalize(decodeURIComponent(url.pathname.slice(base.length))).replace(/^([/\\]|\.\.)+/, '');
  const target = join(root, relative);
  if (!target.startsWith(root)) return notFound();
  if (existsSync(target) && statSync(target).isDirectory()) {
    if (!url.pathname.endsWith('/')) {
      response.writeHead(301, { Location: `${url.pathname}/${url.search}` }).end();
      return;
    }
    const index = join(target, 'index.html');
    return existsSync(index) ? send(200, index) : notFound();
  }
  return existsSync(target) ? send(200, target) : notFound();
}).listen(Number(port), '127.0.0.1', () => console.log(`serving ${root} at http://127.0.0.1:${port}${base}/`));
