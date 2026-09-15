import '../../../../test/setup-dom.ts';
import { afterEach, test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { MessageChannel } from 'node:worker_threads';
import { webcrypto } from 'node:crypto';

// The shipped HTML and production SDK execute against a DOM and real ports.
// The host responses/timers are controlled; this is not a browser/backend E2E.
// Run with Node 24+: node --test ui/src/renderer/pages/agentSession/AgentSessionTemplate.node.ts
// Bun's DOM preload replaces worker_threads MessageChannel; do not run with Bun.
const html = readFileSync(new URL('../../../../../crates/backend/nomifun-app/src/router/plugin_product/templates/agent-session.html', import.meta.url), 'utf8');
const sdk = readFileSync(new URL('../../../../../crates/backend/nomifun-plugin-platform/src/assets/product-sdk.js', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)![1];
type Command = { operation: string; after_seq?: number; limit?: number; input?: { content: string }; idempotency_key?: string };
type KvCommand = { operation: string; key: string; expected_revision?: number; value?: unknown };
type Stored = { value: unknown; revision: number };
const SESSION_A = '0190f5fe-7c00-7a00-8000-0000000000a1';
const SESSION_B = '0190f5fe-7c00-7a00-8000-0000000000b1';
const draftKey = (session = SESSION_A) => 'agent-session/composer/v1/' + session;
type Record = { presentation_intent: string; projection: unknown; hidden?: boolean; message_type?: string; message_status?: string };
const message = (content: string, presentation_intent = 'left'): Record => ({ presentation_intent, projection: { content } });
const observation = (messages = [message('persisted')], seq = 1, status = 'ready', session = SESSION_A) => ({
  session: { agent_session_id: session, metadata: { title: 'Real page fixture' } }, head: { status },
  messages, next_cursor: { seq },
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

function runtime(preview = false, store = new Map<string, Stored>(), session = SESSION_A) {
  const doc = document.implementation.createHTMLDocument('template');
  doc.body.innerHTML = html.match(/<body>([\s\S]*?)<script>/)![1];
  const element = <T extends HTMLElement = HTMLElement>(id: string) => doc.getElementById(id) as T;
  const events = new EventTarget();
  const channel = new MessageChannel();
  let timerId = 0, subscription = 0, sequence = 0;
  const timers = new Map<number, { callback: () => void; delay: number }>();
  const calls: Command[] = [];
  const kvCalls: KvCommand[] = [];
  const protocol: string[] = [];
  let response: (request: Command) => unknown | Promise<unknown> = () => observation(undefined, undefined, undefined, session);
  let storageResponse: (request: KvCommand, execute: () => unknown) => unknown | Promise<unknown> = (_request, execute) => execute();
  const executeKv = (request: KvCommand) => {
    const current = store.get(request.key);
    if (request.operation === 'get') return { outcome: 'value', value: current?.value ?? null, revision: current?.revision };
    if (request.operation !== 'compare_and_swap') throw new Error('Reference must use versioned writes');
    if ((current?.revision ?? null) !== (request.expected_revision ?? null)) {
      return { outcome: 'compare_and_swap', applied: false, current_revision: current?.revision };
    }
    const revision = (current?.revision ?? 0) + 1;
    store.set(request.key, { value: request.value ?? null, revision });
    return { outcome: 'compare_and_swap', applied: true, current_revision: revision };
  };
  const window = Object.assign(events, { __nomifunPluginBridge: channel.port1 });
  const context = { window, document: doc, crypto: webcrypto, TextEncoder, console,
    setTimeout(callback: () => void, delay: number) { timers.set(++timerId, { callback, delay }); return timerId; },
    clearTimeout(id: number) { timers.delete(id); },
  };
  channel.port2.on('message', async request => {
    if (request.type) {
      protocol.push(request.type);
      if (request.type.endsWith('subscribe-v1') && !request.type.endsWith('unsubscribe-v1')) subscription = request.subscription_id;
      return;
    }
    try {
      let result;
      if (request.target.target === 'host_kv') {
        const command = request.target.request as KvCommand;
        kvCalls.push(command);
        result = await storageResponse(command, () => executeKv(command));
      } else {
        const command = request.target.request as Command;
        calls.push(command);
        result = await response(command);
      }
      channel.port2.postMessage({ type: 'nomifun-plugin-bridge-result-v1', call_id: request.call_id, ok: true, result });
    } catch {
      channel.port2.postMessage({ type: 'nomifun-plugin-bridge-result-v1', call_id: request.call_id, ok: false, error: { code: 'UNCONFIRMED', message: 'unconfirmed' } });
    }
  });
  if (preview) Object.defineProperty(window, 'nomi', { value: { preview: true } });
  else runInNewContext(sdk, context);
  runInNewContext(script, context);
  let closed = false;
  const close = () => {
    if (closed) return;
    closed = true;
    events.dispatchEvent(new Event('pagehide'));
    channel.port1.close(); channel.port2.close(); timers.clear();
  };
  closers.push(close);
  return {
    element, calls, kvCalls, store, timers, protocol, events, close,
    respond(fn: typeof response) { response = fn; },
    storageRespond(fn: typeof storageResponse) { storageResponse = fn; },
    click(id: string) { element<HTMLButtonElement>(id).click(); },
    input(text: string) { element<HTMLTextAreaElement>('input').value = text; element('input').dispatchEvent(new Event('input')); },
    tick(delay?: number) {
      const entry = [...timers].find(([, timer]) => delay === undefined || timer.delay === delay);
      if (!entry) throw new Error('No matching timer');
      timers.delete(entry[0]); entry[1].callback();
    },
    async event(kind: 'stream' | 'resync_required', event?: unknown) {
      await until(() => subscription > 0);
      const expectedAcks = protocol.filter(type => type.endsWith('ack-v1')).length + 1;
      channel.port2.postMessage({ type: 'nomifun-plugin-agent-session-event-v1', subscription_id: subscription,
        seq: ++sequence, kind, event, reason: 'fixture reconnect' });
      await until(() => protocol.filter(type => type.endsWith('ack-v1')).length === expectedAcks);
    },
    ready: () => until(() => element('connection').textContent!.startsWith('Session:') &&
      element('draftStatus').dataset.state === 'saved' && !element<HTMLButtonElement>('refresh').disabled),
  };
}

test('preview has no Session calls and gives explicit save/select instructions', () => {
  const r = runtime(true);
  assert.match(r.element('connection').textContent!, /Preview only/);
  assert.equal(r.element<HTMLButtonElement>('send').disabled, true);
  assert.deepEqual(r.calls, []);
  assert.equal(r.timers.size, 0);
});

test('bounded history replaces mutable rows, paginates without dumping IDs and renders text safely', async () => {
  const r = runtime();
  r.respond(() => observation(Array.from({ length: 50 }, (_, i) => message(i === 0 ? '<img src=x onerror=alert(1)>' : 'row ' + i, 'right')), 50));
  await r.ready();
  assert.deepEqual(r.calls[0], { operation: 'observe', after_seq: 0, limit: 50 });
  assert.equal(r.element('history').querySelector('img'), null);
  assert.match(r.element('history').textContent!, /<img src=x/);
  assert.equal(r.element('history').querySelector('strong')?.textContent, 'You');
  r.respond(() => observation([message('updated'), { ...message('hidden'), hidden: true }], 51));
  r.click('next');
  await until(() => r.element('page').textContent === 'Page 2');
  assert.deepEqual(r.calls.at(-1), { operation: 'observe', after_seq: 50, limit: 50 });
  assert.match(r.element('history').textContent!, /updated/);
  assert.doesNotMatch(r.element('history').textContent!, /hidden/);
  assert.equal(r.element<HTMLButtonElement>('next').disabled, true);
  r.respond(() => observation([message('revision at the same cursor')], 51));
  r.click('refresh');
  await until(() => r.element('history').textContent!.includes('revision at the same cursor'));
  assert.equal(r.calls.at(-1)?.after_seq, 50);
  assert.doesNotMatch(r.element('history').textContent!, /updated/);
  r.click('previous');
  await until(() => r.element('page').textContent === 'Page 1');
  assert.equal(r.calls.at(-1)?.after_seq, 0);
});

test('stream/replace/discard/reconnect never splice transient tokens into persisted history or resend', async () => {
  const r = runtime(); await r.ready();
  for (const event of [
    { type: 'content', data: { content: 'TRANSIENT' } },
    { type: 'content', replace: true, data: { content: 'REPLACED' } },
    { type: 'output_discarded', data: { restart_attempt: 1 } },
    { type: 'content', hidden: true, data: { content: 'SECRET' } },
  ]) await r.event('stream', event);
  await r.event('resync_required');
  assert.equal(r.timers.size, 1);
  assert.equal(r.element('history').textContent, 'Agentpersisted');
  r.respond(() => observation([message('durable replacement')], 1));
  r.tick();
  await r.ready();
  assert.equal(r.element('history').textContent, 'Agentdurable replacement');
  assert.deepEqual(r.calls.map(call => call.operation), ['observe', 'observe']);
});

test('source message kinds render text, thinking, plans, tools, notices and Agent status through the production SDK', async () => {
  const r = runtime();
  r.respond(() => observation([
    { ...message('question', 'right'), message_type: 'text' },
    { ...message('reply'), message_type: 'text' },
    { presentation_intent: 'left', message_type: 'thinking', projection: { content: 'considering', status: 'done' } },
    { presentation_intent: 'left', message_type: 'plan', projection: { entries: [
      { content: 'inspect', status: 'completed' }, { content: 'implement', status: 'in_progress' },
    ] } },
    { presentation_intent: 'left', message_type: 'tool_call', projection: {
      name: 'Read', status: 'completed', output: '<img src=x onerror=alert(1)>',
      args: { secret: 'DO_NOT_DUMP_ARGS' }, call_id: 'DO_NOT_DUMP_CALL_ID',
      artifacts: [{ uri: 'javascript:alert(1)' }],
    } },
    { presentation_intent: 'left', message_type: 'tool_group', projection: [
      { name: 'Search', status: 'Executing', result_display: 'searching' },
      { name: 'Read', status: 'Error', result_display: { file_diff: 'DO_NOT_DUMP_DIFF' } },
    ] },
    { ...message('careful'), message_type: 'tips', projection: { content: 'careful', type: 'warning' } },
    { presentation_intent: 'left', message_type: 'agent_status', projection: {
      agent_name: 'Nomi', status: 'connected', session_id: 'DO_NOT_DUMP_SESSION_ID',
    } },
  ], 8));
  await r.ready();
  const history = r.element('history');
  assert.equal(history.querySelectorAll('article').length, 8);
  assert.deepEqual([...history.querySelectorAll('article > strong')].map(node => node.textContent),
    ['You', 'Agent', 'Thinking', 'Plan', 'Tool: Read', 'Tool group', 'Warning', 'Agent status']);
  assert.deepEqual([...history.querySelectorAll('li')].map(node => node.textContent), ['Completed: inspect', 'In progress: implement']);
  assert.deepEqual([...history.querySelectorAll('summary')].map(node => node.textContent), ['Thinking text', 'Output (plain text)', 'Output (plain text)']);
  assert.match(history.textContent!, /<img src=x onerror=alert\(1\)>/);
  assert.match(history.textContent!, /Status: Running/);
  assert.match(history.textContent!, /Status: Error/);
  assert.match(history.textContent!, /Connected/);
  assert.doesNotMatch(history.textContent!, /DO_NOT_DUMP|javascript:/);
  assert.equal(history.querySelectorAll('img, script, a, button, iframe').length, 0);
  assert.deepEqual(r.calls.map(call => call.operation), ['observe']);
});

test('a source error correction replaces tool success at the same cursor without claiming turn success', async () => {
  const r = runtime();
  const tool = { presentation_intent: 'left', message_type: 'tool_call', message_status: 'work',
    projection: { name: 'Read', status: 'completed', output: 'old result' } };
  r.respond(() => observation([tool], 1)); await r.ready();
  assert.match(r.element('history').textContent!, /Status: Completed/);
  r.respond(() => observation([{ ...tool, message_status: 'error', projection: {
    name: 'Read', status: 'completed', error: 'delivery failed', output: 'corrected result',
  } }], 1));
  r.click('refresh');
  await until(() => r.element('history').textContent!.includes('delivery failed'));
  assert.match(r.element('history').textContent!, /Status: Error/);
  assert.doesNotMatch(r.element('history').textContent!, /Completed|old result/);
  assert.equal(r.element('history').querySelectorAll('article').length, 1);
  assert.equal(r.calls.every(call => call.operation === 'observe'), true);
});

test('unknown or malformed records stay explicit, hidden records stay hidden and permission text never becomes a reply', async () => {
  const r = runtime();
  r.respond(() => observation([
    { ...message('DO_NOT_GUESS_PERMISSION'), message_type: 'permission' },
    { ...message('DO_NOT_GUESS_FUTURE_KIND'), message_type: 'future_kind' },
    { presentation_intent: 'left', message_type: 'tool_call', projection: null },
    { presentation_intent: 'left', message_type: 'plan', projection: { entries: 'malformed' } },
    { presentation_intent: 'left', message_type: 'tool_call', projection: { name: 'Read', status: '__proto__' } },
    { ...message('DO_NOT_SHOW_HIDDEN'), hidden: true },
    { ...message('DO_NOT_SHOW_NESTED'), projection: { content: 'DO_NOT_SHOW_NESTED', hidden: true } },
  ], 7));
  await r.ready();
  assert.equal(r.element('history').querySelectorAll('article').length, 5);
  assert.equal(r.element('history').textContent!.match(/Unsupported record/g)?.length, 4);
  assert.match(r.element('history').textContent!, /Unknown status/);
  assert.doesNotMatch(r.element('history').textContent!, /DO_NOT_|Agent|object Object/);
  r.respond(() => observation([{ ...message('hidden'), hidden: true }], 7));
  r.click('refresh');
  await until(() => r.element('history').textContent!.includes('No visible persisted messages'));
  assert.equal(r.calls.every(call => call.operation === 'observe'), true);
});

test('ambiguous send retains exact immutable intent for explicit retry only', async () => {
  const r = runtime(); await r.ready();
  r.respond(command => { if (command.operation === 'turn') throw new Error('unconfirmed'); return observation(); });
  r.input('  one intent  '); r.click('send');
  await until(() => r.element('command').textContent!.includes('unconfirmed') && !r.element<HTMLButtonElement>('refresh').disabled);
  const original = r.calls.find(call => call.operation === 'turn')!;
  assert.deepEqual(original.input, { content: 'one intent' });
  assert.ok(original.idempotency_key);
  assert.equal(r.element<HTMLTextAreaElement>('input').value, '  one intent  ');
  assert.equal(r.element<HTMLTextAreaElement>('input').disabled, true);
  await r.event('resync_required');
  r.tick(); await r.ready();
  assert.equal(r.calls.filter(call => call.operation === 'turn').length, 1);
  r.respond(command => command.operation === 'turn' ? { operation_id: 'accepted' } : observation());
  r.click('retry');
  await until(() => r.element('command').textContent!.includes('Request accepted'));
  assert.deepEqual(r.calls.filter(call => call.operation === 'turn'), [original, original]);
  assert.equal(r.element<HTMLTextAreaElement>('input').value, '');
});

test('SDK timeout is not replayed; abandoning needs inline acknowledgement supported in sandbox', async () => {
  const r = runtime(); await r.ready();
  r.respond(command => command.operation === 'turn' ? new Promise(() => {}) : observation());
  r.input('preserve me'); r.click('send');
  await until(() => r.calls.some(call => call.operation === 'turn'));
  r.tick(15000);
  await until(() => r.element('command').textContent!.includes('unconfirmed'));
  assert.equal(r.calls.filter(call => call.operation === 'turn').length, 1);
  assert.equal(r.element<HTMLButtonElement>('abandon').disabled, true);
  r.element<HTMLInputElement>('acknowledge').checked = true;
  r.element('acknowledge').dispatchEvent(new Event('change'));
  r.click('abandon');
  await until(() => r.element('command').textContent!.includes('Retry identity abandoned'));
  assert.match(r.element('command').textContent!, /Retry identity abandoned/);
  assert.equal(r.element<HTMLTextAreaElement>('input').value, 'preserve me');
  assert.equal(r.calls.filter(call => call.operation === 'turn').length, 1);
});

test('read failures clear old history, stop polling and preserve input until explicit recovery', async () => {
  const r = runtime(); await r.ready(); r.input('unsent');
  r.respond(() => { throw new Error('revoked'); });
  r.click('refresh');
  await until(() => r.element('error').textContent!.includes('paused'));
  assert.equal(r.element('history').textContent, '');
  assert.equal(r.element<HTMLTextAreaElement>('input').value, 'unsent');
  assert.equal(r.timers.size, 0);
  await r.event('resync_required');
  assert.equal(r.timers.size, 0);
  r.respond(() => observation()); r.click('refresh');
  await until(() => !r.element<HTMLButtonElement>('refresh').disabled);
  r.tick(400); await r.ready();
  assert.equal(r.element<HTMLTextAreaElement>('input').value, 'unsent');
  assert.equal(r.calls.every(call => call.operation === 'observe'), true);
});

test('cancel uses existing command, refreshes status, and pagehide releases subscription without cancelling again', async () => {
  const r = runtime();
  r.respond(() => observation([], 0, 'running')); await r.ready();
  r.respond(command => command.operation === 'cancel' ? { canceled: true } : observation([], 0, 'ready'));
  r.click('cancel');
  await until(() => r.element('connection').textContent!.includes('ready'));
  assert.equal(r.calls.filter(call => call.operation === 'cancel').length, 1);
  assert.match(r.element('command').textContent!, /Cancellation requested/);
  r.events.dispatchEvent(new Event('pagehide'));
  await until(() => r.protocol.some(type => type.endsWith('unsubscribe-v1')));
  assert.equal(r.timers.size, 0);
  assert.equal(r.calls.filter(call => call.operation === 'cancel').length, 1);
});

test('an invalid history cursor fails closed instead of looping through a page', async () => {
  const r = runtime();
  r.respond(() => observation([message('invalid cursor')], 0));
  await until(() => r.element('error').textContent!.includes('paused'));
  assert.equal(r.timers.size, 0);
  assert.equal(r.element<HTMLButtonElement>('next').disabled, true);
  assert.equal(r.element('history').textContent, '');
  assert.equal(r.calls.length, 1);
});

test('in-flight observation and queued refresh cannot mutate a disposed page or re-subscribe', async () => {
  const r = runtime(); await r.ready();
  let resolve!: (value: unknown) => void;
  r.respond(() => new Promise(done => { resolve = done; }));
  r.click('refresh');
  await until(() => r.calls.length === 2);
  await r.event('resync_required');
  r.events.dispatchEvent(new Event('pagehide'));
  const before = r.element('history').textContent;
  resolve(observation([message('late result')]));
  await until(() => r.timers.size === 0 && r.protocol.some(type => type.endsWith('unsubscribe-v1')));
  assert.equal(r.element('history').textContent, before);
  assert.equal(r.calls.length, 2);
});

test('failed cancellation stays unconfirmed and polling never retries that mutation', async () => {
  const r = runtime();
  r.respond(() => observation([], 0, 'running')); await r.ready();
  r.respond(command => {
    if (command.operation === 'cancel') throw new Error('unconfirmed');
    return observation([], 0, 'running');
  });
  r.click('cancel');
  await until(() => r.element('command').textContent!.includes('unconfirmed') && !r.element<HTMLButtonElement>('refresh').disabled);
  r.tick(1500);
  await until(() => !r.element<HTMLButtonElement>('refresh').disabled);
  assert.equal(r.calls.filter(call => call.operation === 'cancel').length, 1);
  assert.match(r.element('connection').textContent!, /running/);
});

test('autosaved draft survives a new page, remains Session-scoped and contains no transcript', async () => {
  const shared = new Map<string, Stored>();
  const first = runtime(false, shared); await first.ready();
  first.input('draft to restore'); first.tick(400); await first.ready();
  assert.deepEqual(shared.get(draftKey())?.value, { version: 1, session_id: SESSION_A, text: 'draft to restore', pending: null });
  first.close();
  const second = runtime(false, shared); await second.ready();
  assert.equal(second.element<HTMLTextAreaElement>('input').value, 'draft to restore');
  assert.equal(second.calls.filter(call => call.operation === 'turn').length, 0);
  second.close();
  const otherSession = runtime(false, shared, SESSION_B); await otherSession.ready();
  assert.equal(otherSession.element<HTMLTextAreaElement>('input').value, '');
  assert.deepEqual(otherSession.kvCalls.map(call => call.key), [draftKey(SESSION_B)]);
  assert.equal(shared.size, 1);
});

test('pending intent is durably saved before turn and restored without automatic replay', async () => {
  const shared = new Map<string, Stored>();
  const first = runtime(false, shared); await first.ready();
  let persistedAtSend: unknown;
  first.respond(command => {
    if (command.operation !== 'turn') return observation();
    persistedAtSend = structuredClone(shared.get(draftKey())?.value);
    throw new Error('ambiguous');
  });
  first.input('recover this intent'); first.click('send');
  await until(() => first.element('command').textContent!.includes('unconfirmed'));
  const command = first.calls.find(call => call.operation === 'turn')!;
  assert.deepEqual(persistedAtSend, { version: 1, session_id: SESSION_A, text: 'recover this intent', pending: { key: command.idempotency_key, input: command.input } });
  first.close();
  const second = runtime(false, shared); await second.ready();
  assert.equal(second.calls.filter(call => call.operation === 'turn').length, 0);
  assert.equal(second.element<HTMLTextAreaElement>('input').value, 'recover this intent');
  assert.equal(second.element<HTMLButtonElement>('retry').hidden, false);
  second.respond(request => request.operation === 'turn' ? { operation_id: 'previous' } : observation());
  second.click('retry');
  await until(() => second.element('command').textContent!.includes('Request accepted'));
  assert.deepEqual(second.calls.find(call => call.operation === 'turn'), command);
  assert.deepEqual(shared.get(draftKey())?.value, { version: 1, session_id: SESSION_A, text: '', pending: null });
});

test('write timeout can have committed, but it never sends and explicit reload recovers the same key', async () => {
  const shared = new Map<string, Stored>();
  const r = runtime(false, shared); await r.ready();
  r.storageRespond((command, execute) => {
    if (command.operation === 'get') return execute();
    execute(); // commit with a lost response
    return new Promise(() => {});
  });
  r.input('save before send'); r.click('send');
  await until(() => shared.has(draftKey()));
  r.tick(15000);
  await until(() => r.element('command').textContent!.includes('not sent'));
  assert.equal(r.calls.some(call => call.operation === 'turn'), false);
  assert.equal(r.element('draftStatus').dataset.state, 'error');
  r.storageRespond((_command, execute) => execute());
  await until(() => !r.element<HTMLButtonElement>('refresh').disabled);
  r.element<HTMLInputElement>('restoreAcknowledge').checked = true;
  r.element('restoreAcknowledge').dispatchEvent(new Event('change'));
  r.click('restoreDraft'); await r.ready();
  assert.equal(r.element<HTMLButtonElement>('retry').hidden, false);
  assert.equal(r.calls.some(call => call.operation === 'turn'), false);
  assert.equal(r.kvCalls.filter(call => call.operation === 'compare_and_swap').length, 1);
});

test('late older autosave cannot replace the pending intent; send waits for a second versioned save', async () => {
  const r = runtime(); await r.ready();
  let release: (() => void) | undefined;
  r.storageRespond((command, execute) => {
    if (command.operation === 'get' || release) return execute();
    return new Promise(resolve => { release = () => resolve(execute()); });
  });
  r.input('older text'); r.tick(400);
  await until(() => !!release);
  r.input('latest text'); r.click('send');
  assert.equal(r.calls.some(call => call.operation === 'turn'), false);
  assert.ok(release);
  release();
  await until(() => r.element('command').textContent!.includes('Request accepted'));
  const writes = r.kvCalls.filter(call => call.operation === 'compare_and_swap');
  assert.equal(writes.length, 3); // old draft, pending intent, accepted cleanup
  assert.equal((writes[0].value as { text: string }).text, 'older text');
  assert.equal((writes[1].value as { text: string }).text, 'latest text');
  assert.equal(writes[1].expected_revision, 1);
  assert.deepEqual(r.calls.find(call => call.operation === 'turn')?.input, { content: 'latest text' });
});

test('conflicting writer preserves newer storage and local input; user decides whether to reload', async () => {
  const r = runtime(); await r.ready();
  r.store.set(draftKey(), { value: { version: 1, session_id: SESSION_A, text: 'newer record', pending: null }, revision: 2 });
  r.input('local work'); r.tick(400);
  await until(() => r.element('draftStatus').dataset.state === 'error');
  assert.equal(r.element<HTMLTextAreaElement>('input').value, 'local work');
  assert.equal((r.store.get(draftKey())?.value as { text: string }).text, 'newer record');
  assert.equal(r.element<HTMLButtonElement>('send').disabled, true);
  r.click('restoreDraft'); // still requires explicit acknowledgement
  assert.equal(r.kvCalls.filter(call => call.operation === 'get').length, 1);
  r.element<HTMLInputElement>('restoreAcknowledge').checked = true;
  r.element('restoreAcknowledge').dispatchEvent(new Event('change'));
  r.click('restoreDraft'); await r.ready();
  assert.equal(r.element<HTMLTextAreaElement>('input').value, 'newer record');
  assert.equal(r.calls.some(call => call.operation === 'turn'), false);
});

test('accepted turn with failed cleanup does not masquerade as failed send or issue a new intent', async () => {
  const shared = new Map<string, Stored>();
  const r = runtime(false, shared); await r.ready();
  r.storageRespond((command, execute) => {
    if (command.operation === 'compare_and_swap' && (command.value as { pending: unknown }).pending === null) throw new Error('cleanup failed');
    return execute();
  });
  r.input('already accepted'); r.click('send');
  await until(() => r.element('command').textContent!.includes('Request accepted, but'));
  assert.equal(r.calls.filter(call => call.operation === 'turn').length, 1);
  assert.equal(r.element<HTMLButtonElement>('retry').disabled, true);
  r.close();
  const next = runtime(false, shared); await next.ready();
  assert.equal(next.element<HTMLButtonElement>('retry').hidden, false);
  assert.equal(next.calls.some(call => call.operation === 'turn'), false);
});

test('invalid saved identity is not loaded or overwritten without explicit discard', async () => {
  const shared = new Map<string, Stored>([[draftKey(), { value: { version: 99, session_id: SESSION_B, text: 'wrong Session', pending: null }, revision: 4 }]]);
  const r = runtime(false, shared);
  await until(() => r.element('draftStatus').dataset.state === 'error');
  assert.equal(r.element<HTMLTextAreaElement>('input').value, '');
  assert.equal(r.kvCalls.filter(call => call.operation === 'compare_and_swap').length, 0);
  r.element<HTMLInputElement>('restoreAcknowledge').checked = true;
  r.element('restoreAcknowledge').dispatchEvent(new Event('change'));
  r.click('resetDraft'); await r.ready();
  assert.equal(r.kvCalls.at(-1)?.expected_revision, 4);
  assert.deepEqual(shared.get(draftKey())?.value, { version: 1, session_id: SESSION_A, text: '', pending: null });
});

test('closing while intent is being saved never sends after the late successful write', async () => {
  const r = runtime(); await r.ready();
  let release!: () => void;
  r.storageRespond((command, execute) => command.operation === 'get' ? execute() : new Promise(resolve => { release = () => resolve(execute()); }));
  r.input('leave during save'); r.click('send');
  await until(() => !!release);
  r.events.dispatchEvent(new Event('pagehide'));
  release();
  await until(() => r.timers.size === 0);
  assert.equal(r.calls.some(call => call.operation === 'turn'), false);
});
