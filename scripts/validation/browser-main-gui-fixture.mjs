/**
 * Loopback pages and request witnesses for the real NomiFun main-window UI.
 * This serves website documents only; no renderer, browser toolbar, auth,
 * canonical Session or native browser behavior is mocked here.
 *
 * Run after the owned native candidate is ready:
 *   bun scripts/validation/browser-main-gui-fixture.mjs
 * Read the READY origins and PID; GET /__fixture/status returns bounded facts.
 * It does not change system, process or WebView2 network policy.
 */
import { createServer } from 'node:http';
import { randomUUID } from 'node:crypto';

const counts = new Map();
const witnesses = [];
const timers = new Set();
const sockets = new Set();
let closed = false;
const kinds = new Set(['document', 'iframe', 'image', 'script', 'empty']);
const pages = new Set(['/stable', '/loop', '/aba-a', '/aba-b', '/slow', '/resources', '/frame', '/popup']);

function count(req, path) {
  const rawKind = req.headers['sec-fetch-dest'];
  const kind = typeof rawKind === 'string' && kinds.has(rawKind) ? rawKind : 'unknown';
  const key = `${path}:${kind}`;
  if (counts.has(key) || counts.size < 128) counts.set(key, (counts.get(key) ?? 0) + 1);
}

function document(path, query) {
  const nonce = randomUUID();
  const requestedPeriod = Number(query.get('ms') ?? 700);
  const period = Number.isInteger(requestedPeriod) && requestedPeriod >= 150 && requestedPeriod <= 2000 ? requestedPeriod : 700;
  const loop = path === '/loop'
    ? `setInterval(()=>location.reload(),${period});`
    : path === '/aba-a' || path === '/aba-b'
      ? `setInterval(()=>location.replace('${path === '/aba-a' ? '/aba-b' : '/aba-a'}?ms=${period}'),${period});`
      : '';
  const resources = path === '/resources'
    ? `<iframe title="ordinary fixture frame" src="/frame"></iframe><img alt="fixture resource" src="/image"><script>setInterval(()=>fetch('/tick').catch(()=>{}),100);</script>`
    : '';
  const popup = path === '/popup'
    ? `<button id="popup" onclick="window.open('/stable','fixture-child')">打开普通网页标签</button> <button id="popup-blank" onclick="window.open('/stable','_blank')">打开新网页标签</button>`
    : '';
  const links = path === '/stable'
    ? `<nav aria-label="真实网页导航场景"><a href="/slow?delay_ms=120000">页面发起慢加载</a> · <a href="/loop">页面发起重复导航</a> · <a href="/aba-a">页面发起往返导航</a> · <a href="/resources">普通子资源</a> · <a href="/redirect-a">正常重定向</a> · <a href="/popup">普通弹窗入口</a></nav>`
    : '';
  return `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>NomiFun GUI ${path}</title>
<style>body{font:18px system-ui;margin:24px;line-height:1.6}code{display:block;overflow-wrap:anywhere}button,input{font:inherit;padding:8px}iframe{width:360px;height:100px}</style>
<h1 id="fixture-heading">真实主界面浏览器验收：${path}</h1><p>本页由本次自有 loopback 服务器提供。</p>
<code id="fixture-nonce">${nonce}</code><label>人工输入回显 <input id="echo" oninput="document.getElementById('echo-value').textContent=this.value;fetch('/witness',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({kind:'manual',nonce:window.fixtureNonce,trusted:event.isTrusted})}).catch(()=>{})"></label><output id="echo-value"></output>
${links}${resources}${popup}<script>window.fixtureNonce='${nonce}';${loop}</script></html>`;
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url ?? '/', 'http://127.0.0.1');
  const path = url.pathname;
  res.setHeader('cache-control', 'no-store');
  if (path === '/__fixture/status') {
    res.setHeader('content-type', 'application/json; charset=utf-8');
    res.end(JSON.stringify({ scope: 'owned-loopback-main-gui-pages', pid: process.pid,
      counts: Object.fromEntries(counts), witnesses }));
    return;
  }
  count(req, path);
  if (path === '/redirect-a' || path === '/redirect-b') {
    res.writeHead(302, { location: path === '/redirect-a' ? '/redirect-b' : '/stable' });
    res.end();
    return;
  }
  if (path === '/witness' && req.method === 'POST') {
    let text = '';
    for await (const chunk of req) {
      text += chunk;
      if (text.length > 4096) { res.writeHead(413); res.end(); return; }
    }
    try {
      const value = JSON.parse(text);
      if (value.kind === 'manual' && typeof value.nonce === 'string'
          && /^[a-f0-9-]{36}$/.test(value.nonce) && witnesses.length < 32) {
        witnesses.push({ kind: 'manual', nonce: value.nonce, trusted: value.trusted === true });
      }
    } catch { /* Arbitrary page content never enters the witness output. */ }
    res.writeHead(204); res.end(); return;
  }
  if (path === '/tick' || path === '/image') { res.writeHead(204); res.end(); return; }
  if (!pages.has(path)) { res.writeHead(404); res.end('Fixture route not found'); return; }
  const send = () => {
    if (res.destroyed || res.writableEnded) return;
    res.setHeader('content-type', 'text/html; charset=utf-8');
    res.end(document(path, url.searchParams));
  };
  if (path === '/slow') {
    const requestedDelay = Number(url.searchParams.get('delay_ms') ?? 8000);
    const delay = Number.isInteger(requestedDelay) && requestedDelay >= 1000 && requestedDelay <= 180000 ? requestedDelay : 8000;
    const timer = setTimeout(() => { timers.delete(timer); send(); }, delay);
    timers.add(timer);
    res.on('close', () => { clearTimeout(timer); timers.delete(timer); });
  } else { send(); }
});

server.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
const pageOrigin = `http://127.0.0.1:${server.address().port}`;
console.log(`BROWSER_MAIN_GUI_FIXTURE_READY ${JSON.stringify({ pid: process.pid, page_origin: pageOrigin })}`);

function shutdown() {
  if (closed) return;
  closed = true;
  for (const timer of timers) clearTimeout(timer);
  for (const socket of sockets) socket.destroy();
  server.close();
}
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
