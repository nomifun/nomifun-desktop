// Driver-fixture checks only. These do not claim native Browser acceptance.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdtemp, realpath, rename, rmdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { isAbsolute, join, relative } from 'node:path';
import { Script } from 'node:vm';
import test from 'node:test';
import { createWindowsBrowserScenario } from './global-os-windows-browser-cases.mjs';

test('Windows Browser scenario exposes the public chain and isolated bounded receipt fixture', { skip: process.platform !== 'win32' }, async () => {
  const temp = await realpath(tmpdir()), fixture = await mkdtemp(join(temp, 'nomifun-global-os-product-'));
  // The production allocator uses a UUID, whereas mkdtemp supplies six suffix
  // characters. Rename only this empty test directory to match that contract.
  const allocated = join(temp, `nomifun-global-os-product-${randomUUID()}`);
  await rename(fixture, allocated);
  let scenario;
  try {
    const workspace = join(allocated, 'workspace');
    scenario = await createWindowsBrowserScenario({ fixture: allocated, workspace, loginIdentity: { administrator: false } });
    const html = await fetch(scenario.resources.url).then(response => { assert.equal(response.status, 200); return response.text(); });
    const script = /<script>([\s\S]+)<\/script>/.exec(html)?.[1];
    assert(script); new Script(script); // Parse exactly the source served to WebView2.
    assert(html.includes('type="file" multiple'));
    assert(html.includes('Verify retained files'));
    assert.equal(scenario.steps.filter(step => step.name === 'browser_upload').length, 1);
    assert.equal(scenario.steps.filter(step => step.name === 'browser_act').length, 1);
    assert(scenario.steps.every(step => ['start_process', 'browser_navigate', 'browser_observe', 'browser_upload', 'browser_act', 'delete_path'].includes(step.name)));
    const nonce = /Disposable upload fixture ([0-9a-f-]+)<\/h1>/.exec(html)?.[1]; assert(nonce);
    const target = { tab_id: 'test-only-tab', runtime_generation: 1, document_generation: 2 };
    const element = { reference: { target, observation_generation: 3, ref_id: 'test-only-ref' }, role: 'button', name: 'Global fixture files', focused: false };
    await scenario.steps.find(step => step.id === 'browser-open-fixture').verify({ active_tab_id: target.tab_id, tabs: [{ url: scenario.resources.url, target }] });
    const observation = { target, observation_generation: 3, content: `Disposable upload fixture ${nonce}`, elements: [element], unobserved_frames: 0 };
    const observe = scenario.steps.find(step => step.id === 'browser-observe-upload');
    assert.equal(await observe.verify({ ...observation, content: 'Disposable upload fixture another-nonce' }), false);
    await assert.rejects(observe.verify({ ...observation, target: { ...target, tab_id: 'another-tab' } }), /unique tab/);
    // This is the actual public observe shape: it has no content_url field.
    await observe.verify(observation);
    const input = await scenario.steps.find(step => step.id === 'browser-upload-global').args();
    assert.deepEqual(input.element, element); assert.equal(input.files.length, 3);
    assert(isAbsolute(input.files[0])); assert(relative(workspace, input.files[0]).startsWith('..'));
    assert(!isAbsolute(input.files[1])); assert(input.files[1].startsWith('..'));
    assert(input.files[2].includes('linked-directory'));
    const disposed = scenario.steps.find(step => step.id === 'browser-confirm-disposed');
    assert.equal(await disposed.verify(observation), false);
    await assert.rejects(disposed.verify({ ...observation, content: `Disposable upload document released ${nonce}` }), /different committed document/);
    await disposed.verify({ ...observation, target: { ...target, document_generation: 3 }, content: `Disposable upload document released ${nonce}` });
    assert.equal(await fetch(new URL('/not-this-fixture', scenario.resources.url)).then(response => response.status), 404);
    const receiptPath = /receiptPath=("(?:\\.|[^"\\])+")/.exec(script)?.[1]; assert(receiptPath);
    const rejected = await fetch(new URL(JSON.parse(receiptPath), scenario.resources.url), { method: 'POST', body: JSON.stringify({ nonce: 'not-this-fixture', kind: 'selected', trusted: true, files: [] }) });
    assert.equal(rejected.status, 400);
    await assert.rejects(scenario.steps.find(step => step.id === 'browser-selected-proof').verify({}), /Expected values to be strictly equal/);
  } finally {
    await scenario?.close();
    // No Agent operation ran: this test directory remains its own empty root.
    await rmdir(allocated);
  }
});
