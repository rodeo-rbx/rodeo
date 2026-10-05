// Serves docs/.versions/site/ under /rodeo/, the way GitHub Pages does, to preview
// the output of build.mjs. Usage: `npm run preview:versions [-- <port>]`.

import { createReadStream, existsSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { dirname, extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const BASE = '/rodeo/';
const root = join(dirname(dirname(fileURLToPath(import.meta.url))), '.versions', 'site');
const port = Number(process.argv[2] ?? 4321);
const TYPES = {
	'.html': 'text/html; charset=utf-8',
	'.css': 'text/css',
	'.js': 'text/javascript',
	'.json': 'application/json',
	'.svg': 'image/svg+xml',
	'.png': 'image/png',
	'.jpeg': 'image/jpeg',
	'.webp': 'image/webp',
	'.xml': 'application/xml',
	'.wasm': 'application/wasm',
};

function resolve(urlPath) {
	if (!urlPath.startsWith(BASE)) return null;
	let file = normalize(join(root, decodeURIComponent(urlPath.slice(BASE.length))));
	if (!file.startsWith(root)) return null;
	if (existsSync(file) && statSync(file).isDirectory()) file = join(file, 'index.html');
	return existsSync(file) ? file : null;
}

createServer((req, res) => {
	const path = new URL(req.url, 'http://localhost').pathname;
	if (path === '/') return res.writeHead(302, { location: BASE }).end();
	const file = resolve(path) ?? join(root, '404.html');
	res.writeHead(file.endsWith('404.html') && !path.endsWith('404.html') ? 404 : 200, {
		'content-type': TYPES[extname(file)] ?? 'application/octet-stream',
	});
	createReadStream(file).pipe(res);
}).listen(port, () => console.log(`http://localhost:${port}${BASE}`));
