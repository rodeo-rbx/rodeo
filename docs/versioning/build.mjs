// Builds every docs version and assembles them into one GitHub Pages site:
//
//   /rodeo/        newest stable release (what `mise use ubi:revvy02/rodeo` installs)
//   /rodeo/next/   the working tree (main)
//   /rodeo/vX.Y/   newest patch tag of each older stable minor, back to OLDEST
//
// Tagged versions build from the markdown committed at the tag (cli.md and
// runtime/ are committed at every tag), so they skip `npm run gen` and need only
// node. Each version gets a version switcher in the header and, unless it is the
// newest stable release, a banner pointing at it. Both are injected into the
// built HTML here rather than rendered by Starlight, so tags that predate this
// script get them too.
//
// Run from docs/: `npm run build:versions`. Output: docs/.versions/site/.

import { execSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const SITE = 'https://rodeo-rbx.github.io';
const BASE = '/rodeo/';
// Oldest stable minor to publish. Every stable tag from v1.0.0 on has the docs site.
const OLDEST = [1, 0];
// Shared switcher assets, served from the site root next to the versions.
const ASSETS_DIR = '_versions';
// Link paths (without the leading slash) under a non-root version, and the bare base.
const VERSIONED_PATH = new RegExp(`^${BASE.slice(1)}(next|v\\d+\\.\\d+)/`);
const UNVERSIONED_BASE = new RegExp(`^${BASE.slice(1, -1)}(?=$|[/?#])/?`);

const docsDir = dirname(dirname(fileURLToPath(import.meta.url)));
const repoRoot = execSync('git rev-parse --show-toplevel', { cwd: docsDir, encoding: 'utf8' }).trim();
const workDir = join(docsDir, '.versions');
const srcDir = join(workDir, 'src'); // extracted tags + their node_modules, reused across runs
const outDir = join(workDir, 'out'); // one astro build per version
const siteDir = join(workDir, 'site');

function run(command, cwd) {
	console.log(`$ ${command}`);
	execSync(command, { cwd, stdio: 'inherit', env: { ...process.env, ASTRO_TELEMETRY_DISABLED: '1' } });
}

function stableMinors() {
	const newestPerMinor = new Map();
	for (const tag of execSync('git tag -l "v*"', { cwd: repoRoot, encoding: 'utf8' }).split('\n')) {
		const m = /^v(\d+)\.(\d+)\.(\d+)$/.exec(tag.trim());
		if (!m) continue; // prerelease or not a version tag
		const [major, minor, patch] = m.slice(1).map(Number);
		if (major < OLDEST[0] || (major === OLDEST[0] && minor < OLDEST[1])) continue;
		const key = `${major}.${minor}`;
		const prev = newestPerMinor.get(key);
		if (!prev || patch > prev.patch) newestPerMinor.set(key, { tag: m[0], major, minor, patch });
	}
	return [...newestPerMinor.values()].sort((a, b) => b.major - a.major || b.minor - a.minor);
}

function versions() {
	const [latest, ...older] = stableMinors();
	if (!latest) throw new Error('no stable version tags found (shallow clone? fetch with tags)');
	return [
		{ id: 'next', label: 'next', base: `${BASE}next/` },
		{ id: `v${latest.major}.${latest.minor}`, label: `v${latest.major}.${latest.minor} (latest)`, base: BASE, tag: latest.tag, latest: true },
		...older.map((v) => ({ id: `v${v.major}.${v.minor}`, label: `v${v.major}.${v.minor}`, base: `${BASE}v${v.major}.${v.minor}/`, tag: v.tag })),
	];
}

function build(version) {
	const out = join(outDir, version.id);
	rmSync(out, { recursive: true, force: true });
	const flags = `--base ${version.base} --site ${SITE} --outDir "${out}"`;
	if (!version.tag) {
		// `npm run build` regenerates cli.md and runtime/ first (prebuild).
		run(`npm run build -- ${flags}`, docsDir);
		return out;
	}
	// Tags never move, so an extracted tag with installed dependencies is reused.
	const tagRoot = join(srcDir, version.tag);
	const tagDocs = join(tagRoot, 'docs');
	if (!existsSync(join(tagRoot, '.ready'))) {
		rmSync(tagRoot, { recursive: true, force: true });
		mkdirSync(tagRoot, { recursive: true });
		run(`git archive --format=tar ${version.tag} docs | tar -x -C "${tagRoot}"`, repoRoot);
		run('npm ci --no-audit --no-fund', tagDocs);
		writeFileSync(join(tagRoot, '.ready'), '');
	}
	run(`node_modules/.bin/astro build ${flags}`, tagDocs);
	return out;
}

function htmlFiles(dir) {
	return readdirSync(dir, { recursive: true }).filter((f) => f.endsWith('.html')).map((f) => f.split(sep).join('/'));
}

// The page a built file serves, relative to its version's base: `cli/index.html` -> `cli/`.
// 404.html has no counterpart page in other versions.
function pageOf(file) {
	return file === '404.html' ? null : file.replace(/(^|\/)index\.html$/, '$1');
}

// Where `version`'s copy of `page` lives, falling back to that version's home page.
function target(version, page) {
	const same = page !== null && version.pages.has(page);
	return { href: same ? `${version.base}${page}` : version.base, same };
}

// Markdown links are written as `/rodeo/<page>` and Astro doesn't apply `base` to
// them, so in a version built under another base they would leave the version.
// Root-absolute links missing the `/rodeo/` prefix are broken on this host, so
// they get the version's base too.
function rewriteLinks(html, version) {
	return html.replace(/href="\/(?!\/)([^"]*)"/g, (match, path) => {
		// Already in a specific version: Starlight's own links, or a deliberate cross-version link.
		if (VERSIONED_PATH.test(path)) return match;
		return `href="${version.base}${path.replace(UNVERSIONED_BASE, '')}"`;
	});
}

function replaceOnce(html, pattern, replacement, what, file) {
	let count = 0;
	const out = html.replace(pattern, (...args) => {
		count++;
		return typeof replacement === 'function' ? replacement(...args) : replacement;
	});
	if (count !== 1) throw new Error(`${file}: expected one ${what}, found ${count} (Starlight markup changed?)`);
	return out;
}

function decorate(html, file, version, all, latest) {
	const page = pageOf(file);
	html = rewriteLinks(html, version);

	html = replaceOnce(
		html,
		/<\/head>/g,
		`<link rel="stylesheet" href="${BASE}${ASSETS_DIR}/switcher.css"><script src="${BASE}${ASSETS_DIR}/switcher.js" defer></script></head>`,
		'</head>',
		file,
	);

	const options = all
		.map((v) => {
			if (v === version) return `<option value="${version.base}${page ?? ''}" selected>${v.label}</option>`;
			const t = target(v, page);
			return `<option value="${t.href}"${t.same ? ' data-same-page' : ''}>${v.label}</option>`;
		})
		.join('');
	const select = `<select class="rodeo-version" data-rodeo-version aria-label="Docs version" autocomplete="off">${options}</select>`;
	html = replaceOnce(html, /<a [^>]*class="site-title[^"]*"[^>]*>[\s\S]*?<\/a>/g, (a) => a + select, 'site title', file);

	if (!version.latest) {
		const what = version.tag ? `rodeo ${version.id}` : 'the unreleased version on <code>main</code>';
		const banner =
			`<div class="rodeo-version-banner" data-pagefind-ignore>Docs for ${what}. ` +
			`<a href="${target(latest, page).href}">Go to the docs for the latest release (${latest.tag})</a>.</div>`;
		html = replaceOnce(html, /<main\b[^>]*>/g, (main) => main + banner, '<main>', file);
	}
	return html;
}

const all = versions();
const latest = all.find((v) => v.latest);
console.log(`versions: ${all.map((v) => `${v.id} -> ${v.base}${v.tag ? ` (${v.tag})` : ''}`).join(', ')}`);

for (const version of all) {
	version.out = build(version);
	version.files = htmlFiles(version.out);
	version.pages = new Set(version.files.map(pageOf).filter((p) => p !== null));
}

for (const version of all) {
	for (const file of version.files) {
		const path = join(version.out, file);
		writeFileSync(path, decorate(readFileSync(path, 'utf8'), file, version, all, latest));
	}
}

rmSync(siteDir, { recursive: true, force: true });
cpSync(latest.out, siteDir, { recursive: true });
for (const dir of [...all.filter((v) => !v.latest).map((v) => v.base.slice(BASE.length)), ASSETS_DIR]) {
	if (existsSync(join(siteDir, dir))) throw new Error(`${latest.id} has a page at /${dir}, which collides with a docs version path`);
}
for (const version of all.filter((v) => !v.latest)) {
	cpSync(version.out, join(siteDir, version.base.slice(BASE.length)), { recursive: true });
}
const here = dirname(fileURLToPath(import.meta.url));
mkdirSync(join(siteDir, ASSETS_DIR));
for (const asset of ['switcher.css', 'switcher.js']) cpSync(join(here, asset), join(siteDir, ASSETS_DIR, asset));

console.log(`site: ${relative(process.cwd(), siteDir) || '.'}`);
