import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
import { test } from 'node:test';

const backend = 'http://127.0.0.1:4312';
const syntheticTrust = 'SYNTHETIC_FIXTURE_ONLY';

function initializationScript() {
  const source = readFileSync(new URL('../apps/desktop/src/main.rs', import.meta.url), 'utf8');
  const start = source.indexOf('r#"window.__backendPort = {port};');
  const end = source.indexOf('"#,', start);
  assert.ok(start >= 0 && end > start, 'desktop initialization template must exist');
  return source.slice(start + 3, end)
    .replaceAll('{port}', '4312')
    .replaceAll('{os:?}', '"macos"')
    .replaceAll('{secret:?}', JSON.stringify(syntheticTrust))
    .replaceAll('{{', '{').replaceAll('}}', '}');
}

function fixture(base = backend + '/') {
  const calls = [];
  class OfflineXHR {
    open(method, url) { this.method = method; this.url = url; this.headers = new Headers(); }
    setRequestHeader(name, value) { this.headers.set(name, value); }
    send(body) { calls.push({ transport: 'xhr', input: this.url, body, headers: this.headers }); }
  }
  const window = {
    location: { href: base },
    fetch(input, init) {
      calls.push({ transport: 'fetch', input, init, headers: new Headers(init?.headers ?? input?.headers) });
      return Promise.resolve();
    },
    XMLHttpRequest: OfflineXHR,
  };
  // The VM has no native fetch or network adapter. We execute the actual
  // production template, with a synthetic token and recording-only endpoints.
  vm.runInNewContext(initializationScript(), { window, URL, Headers, Request });
  return { window, calls, OfflineXHR };
}

const cases = [
  `${backend}/api`,
  `${backend}@other.invalid/api`,
  `${backend}0/api`,
  '//other.invalid/api',
  '//127.0.0.1:4312/api',
  '/api',
  'https://other.invalid/api',
  'https://127.0.0.1:4312/api',
  'http://localhost:4312/api',
  `${backend}/api?destination=https://other.invalid`,
  'http://[bad',
  '',
];

for (const base of [backend + '/', 'http://localhost:5173/', 'tauri://localhost/']) {
  test(`fetch and XHR trust the exact parsed backend origin with document base ${base}`, async () => {
    const { window, calls, OfflineXHR } = fixture(base);
    for (const input of cases) {
      let expected = false;
      try { expected = Boolean(input) && new URL(input, base).origin === backend; } catch {}
      await window.fetch(input);
      const xhr = new OfflineXHR(); xhr.open('GET', input); xhr.send();
      assert.equal(calls.length, 2);
      for (const result of calls.splice(0)) {
        assert.equal(result.headers.has('x-nomi-local-trust'), expected,
          `${result.transport} input ${JSON.stringify(input)}`);
      }
    }
  });
}

test('backend fetch keeps a Request body, method, signal and headers while adding trust', async () => {
  const { window, calls } = fixture('http://localhost:5173/');
  const controller = new AbortController();
  const request = new Request(`${backend}/api`, {
    method: 'POST', body: 'fixture-body', signal: controller.signal,
    headers: { 'x-request-header': 'keep', 'content-type': 'text/plain' },
  });
  await window.fetch(request);
  const call = calls[0];
  assert.equal(call.input, request, 'the Request object must not become its URL string');
  assert.equal(call.input.method, 'POST');
  assert.equal(await call.input.clone().text(), 'fixture-body');
  assert.equal(call.input.signal, request.signal);
  assert.equal(call.headers.get('x-request-header'), 'keep');
  assert.equal(call.headers.get('content-type'), 'text/plain');
  assert.equal(call.headers.get('x-nomi-local-trust'), syntheticTrust);
  assert.equal(request.headers.has('x-nomi-local-trust'), false, 'original Request headers stay unchanged');
});

test('explicit fetch init headers preserve Fetch override semantics and other init fields', async () => {
  const { window, calls } = fixture();
  const request = new Request(`${backend}/api`, { headers: { 'x-request-header': 'original' } });
  const init = { method: 'PUT', body: 'override-body', headers: { 'x-init-header': 'override' } };
  await window.fetch(request, init);
  const call = calls[0];
  assert.equal(call.input, request);
  assert.equal(call.init.method, 'PUT');
  assert.equal(call.init.body, 'override-body');
  assert.equal(call.headers.get('x-init-header'), 'override');
  assert.equal(call.headers.has('x-request-header'), false);
  assert.equal(call.headers.get('x-nomi-local-trust'), syntheticTrust);
});

test('fetch preserves backend and external URL objects without converting either to an empty URL', async () => {
  const { window, calls } = fixture();
  for (const url of [new URL(`${backend}/api`), new URL('https://other.invalid/api')]) {
    await window.fetch(url);
    const call = calls.shift();
    assert.equal(call.input, url);
    assert.equal(call.headers.has('x-nomi-local-trust'), url.origin === backend);
  }
});

test('external fetch Request and init objects are untouched', async () => {
  const { window, calls } = fixture();
  const request = new Request('https://other.invalid/api', {
    method: 'POST', body: 'external-fixture', headers: { 'x-request-header': 'keep' },
  });
  const init = { cache: 'no-store' };
  await window.fetch(request, init);
  assert.equal(calls[0].input, request);
  assert.equal(calls[0].init, init);
  assert.deepEqual(init, { cache: 'no-store' });
  assert.equal(calls[0].headers.get('x-request-header'), 'keep');
  assert.equal(calls[0].headers.has('x-nomi-local-trust'), false);
});

test('a plain object cannot earn trust with a spoofed url field before Fetch string conversion', async () => {
  const { window, calls } = fixture();
  const input = { url: `${backend}/api`, headers: { 'x-object-header': 'untrusted' },
    toString: () => 'https://other.invalid/api' };
  await window.fetch(input);
  assert.equal(calls[0].input, input);
  assert.equal(String(calls[0].input), 'https://other.invalid/api');
  assert.equal(calls[0].headers.has('x-nomi-local-trust'), false);
});

test('explicit existing trust headers are retained by both wrappers', async () => {
  const { window, calls, OfflineXHR } = fixture();
  const existing = 'SYNTHETIC_CALLER_HEADER';
  await window.fetch(`${backend}/api`, { headers: { 'x-nomi-local-trust': existing } });
  const xhr = new OfflineXHR(); xhr.open('POST', `${backend}/api`);
  xhr.setRequestHeader('X-Nomi-Local-Trust', existing); xhr.send('fixture-body');
  assert.equal(calls[0].headers.get('x-nomi-local-trust'), existing);
  assert.equal(calls[1].headers.get('x-nomi-local-trust'), existing);
  assert.equal(calls[1].body, 'fixture-body');
});
