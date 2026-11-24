import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { runInNewContext } from 'node:vm';

const web = 'web';
const read = path => readFileSync(join(web, path), 'utf8');
const asset = path => assert.ok(statSync(join(web, path)).isFile(), path);

test('browser JavaScript parses', () => {
  for (const file of readdirSync(join(web, 'js')).filter(name => name.endsWith('.js'))) {
    execFileSync(process.execPath, ['--input-type=module', '--check'], { input: read('js/' + file) });
  }
  execFileSync(process.execPath, ['--check', join(web, 'sw.js')]);
});

test('HTML entry points and manifest icons exist', () => {
  const html = read('index.html');
  for (const file of ['js/app.js', 'app.css', 'manifest.webmanifest']) {
    assert.ok(html.includes('"' + file + '"'), file);
    asset(file);
  }
  const manifest = JSON.parse(read('manifest.webmanifest'));
  assert.ok(manifest.icons.length > 0);
  for (const { src } of manifest.icons) asset(src);
});

function worker(caches) {
  const handlers = new Map();
  runInNewContext(read('sw.js'), {
    caches, URL, location: { origin: 'https://example.test' },
    Request: class { constructor(url) { this.url = url; } },
    self: { addEventListener: (event, handler) => handlers.set(event, handler), skipWaiting() {} },
  });
  return handlers;
}

test('service worker installs the complete offline shell', async () => {
  const cached = new Set();
  const handlers = worker({ open: async () => ({
    addAll: async requests => {
      for (const { url } of requests) {
        if (url !== './') asset(url);
        cached.add(url);
      }
    },
  }) });
  let install;
  handlers.get('install')({ waitUntil: promise => { install = promise; } });
  await install;
  for (const file of ['index.html', 'app.css', 'js/app.js']) assert.ok(cached.has(file), file);
});

test('service worker leaves API, images and external requests to the network', () => {
  const fetchHandler = worker({}).get('fetch');
  for (const url of ['https://example.test/api/status', 'https://example.test/img/cover', 'https://other.test/']) {
    fetchHandler({ request: { method: 'GET', url }, respondWith: () => assert.fail(url) });
  }
});

test('GitLab update check follows pages and selects the platform APK by release path', async () => {
  const pages = [];
  const url = 'https://gitlab.com/Asgefka/Ai_test/-/releases/mobile-v1.2.3/downloads/VNShelf-1.2.3.apk';
  const context = {
    fetch: async address => {
      pages.push(new URL(address).searchParams.get('page'));
      return { ok: true, status: 200, json: async () => pages.length === 1
        ? Array(100).fill({ tag_name: 'pc-v1.2.3' })
        : [{ tag_name: 'mobile-v2.0.0', upcoming_release: true },
           { tag_name: 'mobile-v1.2.3', assets: { links: [{ direct_asset_url: url }] } }] };
    },
  };
  const latest = runInNewContext(read('js/releases.js').replace('export async function', 'async function') + '\nlatestRelease', context);
  assert.equal((await latest('mobile')).url, url);
  assert.deepEqual(pages, ['1', '2']);
  context.fetch = async () => ({ ok: false, status: 404 });
  await assert.rejects(latest('mobile'), /войдите в GitLab/);
});
