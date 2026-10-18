// Public default-Agent -> canonical Browser -> real native WebView2 upload.
// The host serves only this unique loopback page and observes its receipts.
// File creation, upload, source changes, page disposal and deletion use tools.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { createServer } from 'node:http';
import { lstat, readFile, stat } from 'node:fs/promises';
import { basename, join, relative } from 'node:path';
import { assertWindowsFixture, powershell, psArguments, psLiteral } from './global-os-windows-cases.mjs';
import { createWindowsBrowserSnapshotObserver } from './global-os-windows-browser-snapshots.mjs';

const delay = ms => new Promise(done => setTimeout(done, ms));
const absent = async path => lstat(path).then(() => false, error => { if (error.code === 'ENOENT') return true; throw error; });
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const exited = value => { assert.equal(value.state, 'exited'); assert.equal(value.exit_code, 0); assert.equal(value.cleanup?.reaped, true); };

function fixturePage(nonce, receiptPath) {
  return `<!doctype html><html><meta charset="utf-8"><title>NomiFun unique upload ${nonce}</title>
<style>body{font:18px system-ui;margin:36px}input,button{display:block;margin:18px 0;padding:12px;font:inherit}</style>
<h1>Disposable upload fixture ${nonce}</h1><label for="files">Global fixture files</label><input id="files" type="file" multiple>
<button id="verify" type="button">Verify retained files</button><output id="status">Waiting for fixture files</output>
<script>
const nonce=${JSON.stringify(nonce)},receiptPath=${JSON.stringify(receiptPath)};
let retained=[],phase=0;window.fixtureRetainedFiles=retained;
async function describe(file){const bytes=await file.arrayBuffer();const hash=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),x=>x.toString(16).padStart(2,'0')).join('');return {name:file.name,size:file.size,type:file.type,lastModified:file.lastModified,sha256:hash,text:new TextDecoder('utf-8',{fatal:true}).decode(bytes)}}
async function report(kind,event){try{const files=await Promise.all(retained.map(describe));const response=await fetch(receiptPath,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({nonce,kind,trusted:event.isTrusted,files})});if(!response.ok)throw Error('receipt rejected');document.getElementById('status').textContent=kind==='selected'?'Initial native file proof ready':'Retained native file proof ready'}catch(error){document.getElementById('status').textContent='Fixture error: '+error.message}}
document.getElementById('files').addEventListener('change',event=>{if(phase!==0)return;phase=1;retained=Array.from(event.target.files);window.fixtureRetainedFiles=retained;report('selected',event)});
document.getElementById('verify').addEventListener('click',event=>{if(phase!==1)return;phase=2;report('retained',event)});
</script></html>`;
}

export async function createWindowsBrowserScenario({ fixture, workspace, loginIdentity }) {
  await assertWindowsFixture(fixture, workspace);
  assert.equal(loginIdentity.administrator, false);
  const nonce = randomUUID(), root = join(fixture, `browser-cases-${nonce}`), sources = join(root, 'sources'), link = join(root, 'linked-directory');
  const texts = [`NomiFun absolute 中文 ${nonce}\n`, `NomiFun relative 空格 ${nonce}\n`, `NomiFun junction 文件 ${nonce}\n`];
  const actualSources = ['absolute 中文.txt', 'relative 空格.txt', 'linked-文件.txt'].map(name => join(sources, name));
  const files = [actualSources[0], relative(workspace, actualSources[1]), join(link, basename(actualSources[2]))];
  const lastModified = Date.parse('2024-01-02T03:04:05.000Z');
  const expected = actualSources.map((path, index) => ({ name: basename(path), size: Buffer.byteLength(texts[index]), type: 'text/plain', lastModified, sha256: sha256(texts[index]), text: texts[index] }));
  const nativeEvidence = [], receipts = new Map(), receiptPath = `/receipt-${nonce}`, pagePath = `/fixture-${nonce}`, disposalPath = `/disposed-${nonce}`;
  const snapshotObserver = createWindowsBrowserSnapshotObserver({ fixtureNonce: nonce, expected });
  const resources = { root, url: undefined, disposal_url: undefined, source_paths: actualSources };
  let serverError, uploadElement, verifyElement, target, openedTabId, navigationTarget, pageDisposed = false, stopped = false, linkPrepared = false, linkRemoved = false;
  const server = createServer(async (request, response) => {
    try {
      if (request.method === 'GET' && request.url === pagePath) {
        response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' });
        response.end(fixturePage(nonce, receiptPath)); return;
      }
      if (request.method === 'GET' && request.url === disposalPath) {
        response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' });
        response.end(`<!doctype html><meta charset="utf-8"><title>Disposed ${nonce}</title><p>Disposable upload document released ${nonce}</p>`); return;
      }
      if (request.method === 'POST' && request.url === receiptPath) {
        let length = 0; const parts = [];
        for await (const part of request) { length += part.length; assert(length <= 65536, 'The disposable page receipt exceeded its bound'); parts.push(part); }
        const value = JSON.parse(Buffer.concat(parts).toString('utf8'));
        assert.equal(value.nonce, nonce); assert(['selected', 'retained'].includes(value.kind));
        assert(!receipts.has(value.kind), 'A fixture phase must have a single receipt');
        assert.equal(value.trusted, true, 'The native file chooser/click must produce a trusted page event');
        assert.deepEqual(value.files, expected);
        receipts.set(value.kind, value); response.writeHead(204); response.end(); return;
      }
      response.writeHead(404); response.end();
    } catch (error) { serverError ??= error; response.writeHead(400); response.end('Disposable fixture receipt rejected'); }
  });
  await new Promise((done, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', done); });
  const url = `http://127.0.0.1:${server.address().port}${pagePath}`;
  const disposalUrl = new URL(disposalPath, url).href;
  Object.assign(resources, { url, disposal_url: disposalUrl });
  const close = async () => { if (stopped) return; stopped = true; await new Promise((done, reject) => { server.close(error => error ? reject(error) : done()); server.closeAllConnections(); }); };
  const execStep = (id, script, verify) => ({ id, name: 'start_process', args: { command: powershell, args: psArguments(script), timeout_ms: 20000 }, verify: async value => { exited(value); await verify?.(value); } });
  const ownedObservation = value => {
    assert(value.target && Number.isSafeInteger(value.observation_generation));
    assert.equal(value.target.tab_id, openedTabId, 'Read fresh elements only from the unique tab opened by the canonical navigation');
    assert.equal(value.target.runtime_generation, navigationTarget.runtime_generation);
    assert(value.target.document_generation > 0 && value.target.document_generation >= navigationTarget.document_generation, 'The unique fixture document must have committed for the acknowledged new tab');
    assert(value.content.includes(`Disposable upload fixture ${nonce}`), 'The committed document must expose this fixture nonce');
    assert.equal(value.unobserved_frames, 0);
    if (target) assert.deepEqual(value.target, target, 'The upload proof must stay in its original native document');
    target = value.target;
  };
  const observe = (id, receiptKind) => ({ id, name: 'browser_observe', args: {}, repeatable: true, maxAttempts: 20, verify: async value => {
    if (serverError) throw serverError;
    if (typeof value.content !== 'string' || !value.content.includes(`Disposable upload fixture ${nonce}`)) { await delay(200); return false; }
    const find = name => { const elements = value.elements.filter(element => element.name === name); assert.equal(elements.length, 1, `Expected exactly one fresh ${name} element`); return elements[0]; };
    if (!receiptKind) {
      if (!value.elements.some(element => element.name === 'Global fixture files')) { await delay(200); return false; }
      ownedObservation(value); uploadElement = find('Global fixture files'); return;
    }
    ownedObservation(value);
    if (!receipts.has(receiptKind)) { await delay(200); return false; }
    if (receiptKind === 'selected') verifyElement = find('Verify retained files');
    nativeEvidence.push({ step: id, kind: receiptKind === 'selected' ? 'actual_webview2_file_selection' : 'actual_webview2_retained_files', target, observation_generation: value.observation_generation, receipt: receipts.get(receiptKind), source_paths: files });
  } });
  const prepare = `$root=${psLiteral(root)};if(Test-Path -LiteralPath $root){throw 'Fixture collision'};New-Item -ItemType Directory -Path ${psLiteral(sources)}|Out-Null;` +
    actualSources.map((path, index) => `[IO.File]::WriteAllText(${psLiteral(path)},${psLiteral(texts[index])},(New-Object Text.UTF8Encoding($false)));[IO.File]::SetLastWriteTimeUtc(${psLiteral(path)},[DateTime]::Parse('2024-01-02T03:04:05Z').ToUniversalTime());`).join('\n') +
    `New-Item -ItemType Junction -Path ${psLiteral(link)} -Target ${psLiteral(sources)}|Out-Null;[Console]::WriteLine('BROWSER_SOURCE_FIXTURE_READY')`;
  const verifyFixture = async () => {
    assert((await lstat(link)).isSymbolicLink());
    linkPrepared = true;
    for (const [index, path] of actualSources.entries()) { const info = await stat(path); assert.equal(info.size, expected[index].size); assert.equal(Math.floor(info.mtimeMs), lastModified); assert.equal(await readFile(path, 'utf8'), texts[index]); }
    nativeEvidence.push({ step: 'browser-source-fixture', kind: 'independent_external_source_metadata', default_directory: workspace, sources: actualSources, paths_submitted_to_browser: files, expected });
  };
  const dispose = id => ({ id, name: 'browser_navigate', args: { url: disposalUrl }, verify: value => {
    assert(Array.isArray(value.tabs)); const tab = value.tabs.find(tab => tab.target.tab_id === openedTabId);
    assert(tab); assert.equal(value.active_tab_id, openedTabId); assert.equal(tab.url, disposalUrl);
  } });
  const disposed = id => ({ id, name: 'browser_observe', args: {}, repeatable: true, maxAttempts: 20, verify: async value => {
    if (typeof value.content !== 'string' || !value.content.includes(`Disposable upload document released ${nonce}`)) { await delay(200); return false; }
    assert.equal(value.target.tab_id, openedTabId);
    assert.equal(value.target.runtime_generation, navigationTarget.runtime_generation);
    assert(value.target.document_generation > (target?.document_generation ?? navigationTarget.document_generation), 'A different committed document must own the disposed fixture tab');
    assert.equal(value.unobserved_frames, 0);
    pageDisposed = true; nativeEvidence.push({ step: id, kind: 'fixture_document_disposed', target: value.target });
  } });
  const cleanup = id => ({ id, name: 'delete_path', args: { path: root }, verify: async () => assert(await absent(root)) });
  const sourceProof = async () => Promise.all(actualSources.map(async path => {
    if (await absent(path)) return { path, absent: true };
    const info = await stat(path), bytes = await readFile(path);
    return { path, size: info.size, mtime_ms: info.mtimeMs, sha256: sha256(bytes) };
  }));
  const unlink = id => {
    let before;
    return { id, name: 'delete_path', args: async () => {
      assert((await lstat(link)).isSymbolicLink()); before = await sourceProof(); return { path: link };
    }, verify: async () => {
      assert(await absent(link)); assert((await stat(sources)).isDirectory()); assert.deepEqual(await sourceProof(), before);
      linkRemoved = true;
      nativeEvidence.push({ step: id, kind: 'canonical_junction_unlink_target_preserved', link, source_directory: sources, before, after: await sourceProof() });
    } };
  };
  const steps = [execStep('browser-source-fixture', prepare, verifyFixture),
    { id: 'browser-open-fixture', name: 'browser_navigate', args: { url, new_tab: true }, verify: value => {
      assert(Array.isArray(value.tabs)); const tab = value.tabs.find(tab => tab.target.tab_id === value.active_tab_id); assert(tab); openedTabId = tab.target.tab_id;
      assert.equal(tab.url, url); navigationTarget = tab.target;
      // Dispatch acknowledges navigation before the native page necessarily
      // commits. The following bounded observation proves the new document,
      // unique visible fixture nonce and fresh references in this same tab.
    } },
    observe('browser-observe-upload'),
    { id: 'browser-upload-global', name: 'browser_upload', args: async () => {
      nativeEvidence.push({ kind: 'native_upload_snapshot_baseline', receipt: await snapshotObserver.beforeUpload() });
      return { element: uploadElement, files };
    }, verify: async value => {
      assert.deepEqual(value.target, target);
      const receipt = await snapshotObserver.afterUpload();
      resources.upload_snapshot = receipt;
      nativeEvidence.push({ kind: 'native_upload_snapshot_metadata', target, receipt });
    } },
    observe('browser-selected-proof', 'selected'),
    execStep('browser-original-sources-change', `[IO.File]::WriteAllText(${psLiteral(actualSources[0])},'SOURCE_CHANGED_AFTER_UPLOAD',(New-Object Text.UTF8Encoding($false)));Remove-Item -LiteralPath ${psLiteral(actualSources[1])},${psLiteral(actualSources[2])};[Console]::WriteLine('BROWSER_ORIGINALS_CHANGED')`, async () => {
      assert.equal(await readFile(actualSources[0], 'utf8'), 'SOURCE_CHANGED_AFTER_UPLOAD'); assert(await absent(actualSources[1])); assert(await absent(actualSources[2]));
      nativeEvidence.push({ step: 'browser-original-sources-change', kind: 'independent_originals_changed_and_deleted', changed: actualSources[0], deleted: actualSources.slice(1) });
    }),
    // File tools do not replace a fresh page observation before the next input.
    observe('browser-observe-retained-click', 'selected'),
    { id: 'browser-read-retained-files', name: 'browser_act', args: () => ({ action: 'click', element: verifyElement }), verify: value => { assert.deepEqual(value.target, target); } },
    observe('browser-retained-proof', 'retained'), dispose('browser-dispose-page'), disposed('browser-confirm-disposed'), unlink('browser-delete-junction'), cleanup('browser-delete-sources')];
  return { steps, resources, nativeEvidence, close,
    prompt: `只操作本次唯一 ${root} 和本机页面 ${url}。由Agent exec准备三个工作区外文本源及junction和固定mtime，再通过正式Browser navigate/observe fresh ref/upload提交绝对、合法..、junction路径；页面保留原生File并回传本次knownfixture metadata/bytes/hash。Agent改写第一个源、删除另两个源后fresh observe和真实点击Verify retained files，确认网页仍读到原副本。最后navigate本次独立空页面 ${disposalUrl} 并确认新document已替换原页面，再删除唯一root。不触碰其他网页、用户文件或profile，不使用host上传、HTTP伪造文件或DOM合成input/change。`,
    completionSummary: role => role === 'main' ? `本次默认Agent通过真实WebView2上传三个工作区外原生文件，绝对、合法..和junction来源全部到达实际file input；受信change回执的File.name/type/size/lastModified/UTF-8内容和SHA-256匹配。原源改写/删除后，独立受信点击再次读取保留的File仍与原副本一致。独立空页面的新document已替换原文档=${pageDisposed}，本次源目录已删除；snapshot最终释放另由native runtime关闭证明。本地scripted provider不代表联网模型推理。` : `原Browser验收失败保留。本次新Turn只销毁唯一fixture页面并清理 ${root}；不将恢复清理当作首次验收通过。`,
    cleanupNeeded: async () => Boolean(openedTabId && !pageDisposed) || !await absent(root),
    get cleanupSteps() { return [...(openedTabId && !pageDisposed ? [dispose('browser-recovery-dispose'), disposed('browser-recovery-disposed')] : []), ...(linkPrepared && !linkRemoved ? [unlink('browser-recovery-unlink')] : []), cleanup('browser-recovery-delete')]; },
    cleanupPrompt: `原Browser验收失败保持失败；原Turn已取消且canonical effect pending/unknown与原生清理已结算后，只navigate到本次独立空页面 ${disposalUrl} 销毁fixture文档，再delete_path清理唯一 ${root}。不上传其他文件或操作其他网页。`,
    cleanupIntent: { temporary_process: () => undefined, path: root, local_page: url, only_after_canonical_settlement: true },
    pendingAcceptance: [{ id: 'unc-external-volume-upload', status: 'not_run', reason: 'This actual WebView2 case covers its TEMP volume, absolute/parent/junction sources. D-volume/ACL/alias/limit source tests are independent lower-level evidence; UNC and mapped/network volumes require their own native product cases.' },
      { id: 'runtime-snapshot-release', status: 'requires_native_teardown', reason: 'Navigating away proves document disposal. Host-retained upload snapshots are released only after actual native Browser runtime destruction; observe that separately during App teardown.' }],
  };
}
