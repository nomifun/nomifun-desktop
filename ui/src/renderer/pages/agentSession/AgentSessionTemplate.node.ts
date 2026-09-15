import '../../../../test/setup-dom.ts';
import { afterEach, test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { MessageChannel } from 'node:worker_threads';
import { webcrypto } from 'node:crypto';

// Execute the shipped template + SDK with real ports and controlled host/timers.
const html = readFileSync(new URL('../../../../../crates/backend/nomifun-app/src/router/plugin_product/templates/agent-session.html', import.meta.url), 'utf8');
const sdk = readFileSync(new URL('../../../../../crates/backend/nomifun-plugin-platform/src/assets/product-sdk.js', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)![1];
type Command = { operation: string; after_seq?: number; limit?: number; input?: { content: string }; idempotency_key?: string };
const message = (content: string) => ({ presentation_intent: 'left', projection: { content } });
const observation = (messages: unknown[] = [message('persisted')], seq = 1, status = 'ready') => ({
  session: { agent_session_id: 'session-a', metadata: { title: 'Fixture' } }, head: { status }, messages, next_cursor: { seq },
});
const closers: Array<() => void> = [];
afterEach(() => { for (const close of closers.splice(0)) close(); });
async function until(predicate: () => boolean) {
  const end = Date.now() + 2500;
  while (!predicate()) {
    if (Date.now() > end) throw new Error('Template condition timed out');
    await new Promise(resolve => setTimeout(resolve, 1));
  }
}
function runtime(preview = false) {
  const doc = document.implementation.createHTMLDocument('template');
  doc.body.innerHTML = html.match(/<body>([\s\S]*?)<script>/)![1];
  const element = <T extends HTMLElement = HTMLElement>(id: string) => doc.getElementById(id) as T;
  const events = new EventTarget(), channel = new MessageChannel();
  const timers = new Map<number, { callback: () => void; delay: number }>();
  const calls: Command[] = [], other: unknown[] = [];
  let timerId = 0;
  let response: (request: Command) => unknown | Promise<unknown> = () => observation();
  const window = Object.assign(events, { __nomifunPluginBridge: channel.port1 });
  const context = { window, document: doc, crypto: webcrypto, TextEncoder, console,
    setTimeout(callback: () => void, delay: number) { timers.set(++timerId, { callback, delay }); return timerId; },
    clearTimeout(id: number) { timers.delete(id); },
  };
  channel.port2.on('message', async request => {
    if (request.type || request.target?.target !== 'agent_session') { other.push(request); return; }
    const command = request.target.request as Command;
    calls.push(command);
    try {
      const result = await response(command);
      channel.port2.postMessage({ type: 'nomifun-plugin-bridge-result-v1', call_id: request.call_id, ok: true, result });
    } catch {
      channel.port2.postMessage({ type: 'nomifun-plugin-bridge-result-v1', call_id: request.call_id, ok: false, error: { code: 'UNCONFIRMED' } });
    }
  });
  if (preview) Object.defineProperty(window, 'nomi', { value: { preview: true } });
  else runInNewContext(sdk, context);
  runInNewContext(script, context);
  closers.push(() => { events.dispatchEvent(new Event('pagehide')); channel.port1.close(); channel.port2.close(); timers.clear(); });
  return { element, calls, other, timers, events,
    respond(fn: typeof response) { response = fn; },
    click(id: string) { element<HTMLButtonElement>(id).click(); },
    send(text: string) {
      element<HTMLTextAreaElement>('input').value = text;
      element('input').dispatchEvent(new Event('input'));
      element('composer').dispatchEvent(new Event('submit', { cancelable: true }));
    },
    tick() {
      const entry = [...timers].find(([, timer]) => timer.delay !== 15000);
      assert.ok(entry, 'poll timer exists'); timers.delete(entry[0]); entry[1].callback();
    },
    ready: () => until(() => element('connection').textContent!.startsWith('Session:') && !element<HTMLButtonElement>('refresh').disabled),
  };
}

test('preview is inert and explains explicit save/select', () => {
  const r = runtime(true);
  assert.match(r.element('connection').textContent!, /Preview only.*Save.*select/);
  assert.equal(r.element<HTMLButtonElement>('send').disabled, true);
  assert.equal(r.calls.length, 0); assert.equal(r.timers.size, 0);
});

test('polling replaces mutable history, paginates, hides private rows and renders text safely', async () => {
  const r = runtime(); await r.ready();
  r.respond(() => observation(Array.from({ length: 50 }, () => message('<img src=x onerror=alert(1)>')), 50));
  r.tick(); await until(() => r.element('history').querySelectorAll('article').length === 50);
  assert.equal(r.element('history').querySelector('img'), null);
  r.respond(() => observation([
    message('later'), { ...message('hidden'), hidden: true },
    { ...message('private tool payload'), message_type: 'tool_call' },
  ], 53));
  r.click('next'); await until(() => r.element('page').textContent === 'Page 2');
  assert.equal(r.calls.at(-1)?.after_seq, 50);
  assert.match(r.element('history').textContent!, /later.*built-in/);
  assert.doesNotMatch(r.element('history').textContent!, /hidden|private tool payload/);
  r.respond(() => observation([message('corrected')], 53));
  r.tick(); await until(() => r.element('history').textContent!.includes('corrected'));
  assert.doesNotMatch(r.element('history').textContent!, /later/);
  assert.deepEqual(r.other, [], 'no storage, subscriptions or ACK messages');
});

test('send uses host turn once, then observes; input stays local with no durable draft', async () => {
  const r = runtime(); await r.ready(); r.send('hello');
  await until(() => r.calls.length === 3);
  assert.deepEqual(r.calls.map(call => call.operation), ['observe', 'turn', 'observe']);
  assert.deepEqual(r.calls[1].input, { content: 'hello' });
  assert.ok(r.calls[1].idempotency_key);
  await r.ready(); assert.equal(r.element<HTMLTextAreaElement>('input').value, '');
  assert.deepEqual(r.other, []);
});

test('unconfirmed send retains text and stops further sends; polls never replay mutations', async () => {
  const r = runtime(); await r.ready();
  r.respond(command => { if (command.operation === 'turn') throw new Error('offline'); return observation(); });
  r.send('one intent'); await until(() => r.element('command').textContent!.includes('unconfirmed'));
  await r.ready(); r.tick(); await until(() => r.calls.length === 4); await r.ready();
  r.send('one intent');
  assert.equal(r.calls.filter(call => call.operation === 'turn').length, 1);
  assert.equal(r.element<HTMLTextAreaElement>('input').value, 'one intent');
  assert.equal(r.element<HTMLButtonElement>('send').disabled, true);
  assert.deepEqual(r.other, []);
});

test('invalid observation or revoked access clears history and pauses polling until explicit refresh', async () => {
  const r = runtime(); await r.ready();
  r.respond(() => observation([message('invalid')], 0)); r.tick();
  await until(() => r.element('connection').textContent === 'Session unavailable');
  assert.equal(r.element('history').textContent, ''); assert.equal(r.timers.size, 0);
  r.respond(() => observation()); r.click('refresh'); await r.ready();
  assert.match(r.element('history').textContent!, /persisted/);
});

test('cancel uses host API without automatic retry, pagehide stops polling without canceling', async () => {
  const r = runtime(); await r.ready();
  r.respond(command => { if (command.operation === 'cancel') throw new Error('offline'); return observation(undefined, 1, 'running'); });
  r.tick(); await until(() => !r.element<HTMLButtonElement>('cancel').disabled);
  r.click('cancel'); await until(() => r.element('command').textContent!.includes('unconfirmed'));
  await r.ready(); r.tick(); await r.ready();
  r.events.dispatchEvent(new Event('pagehide'));
  assert.equal(r.calls.filter(call => call.operation === 'cancel').length, 1);
  assert.equal(r.timers.size, 0);
});

test('late observation cannot mutate a disposed page or restart polling', async () => {
  const r = runtime(); await r.ready();
  let release!: (value: unknown) => void;
  r.respond(() => new Promise(resolve => { release = resolve; }));
  r.tick(); await until(() => !!release);
  r.events.dispatchEvent(new Event('pagehide'));
  release(observation([message('late')]));
  await until(() => r.timers.size === 0);
  assert.doesNotMatch(r.element('history').textContent!, /late/);
  assert.deepEqual(r.other, []);
});
