import { readFileSync } from 'node:fs';
import { expect, test } from 'bun:test';

const install = new Function('core', readFileSync(new URL('./native_stability.js', import.meta.url), 'utf8'));

function fixture(options: { moving?: boolean; frames?: boolean; blocked?: string; stalled?: boolean } = {}) {
  let now = 0, next = 0;
  const jobs = new Map<number, { at: number; run: () => void }>();
  const schedule = (run: () => void, delay: number) => { const id = ++next; jobs.set(id, { at: now + delay, run }); return id; };
  const node = { isConnected: true, getBoundingClientRect: () => ({ x: options.moving ? now : 10, y: 20, width: 80, height: 30 }) };
  let target = node;
  const calls: string[][] = [];
  const core = {
    retarget: () => target,
    async checkElementStates(_node: unknown, states: string[]) { calls.push(states); return options.blocked ? { missingState: options.blocked } : undefined; },
    utils: { builtins: {
      performance: { now: () => now },
      setTimeout: (run: () => void, delay: number) => schedule(run, options.stalled && delay < 2000 ? Infinity : delay), clearTimeout: (id: number) => jobs.delete(id),
      requestAnimationFrame: (run: () => void) => schedule(run, options.frames ? 16 : Infinity),
      cancelAnimationFrame: (id: number) => jobs.delete(id),
    } },
    __nomiCheckStates: null as unknown as (node: unknown, states: string[]) => Promise<unknown>,
  };
  install(core);
  const advance = async (until: number) => {
    await Promise.resolve();
    for (;;) {
      const job = [...jobs].filter(([, job]) => job.at <= until).sort((a, b) => a[1].at - b[1].at)[0];
      if (!job) break;
      jobs.delete(job[0]); now = job[1].at; job[1].run();
      await Promise.resolve();
    }
    now = until;
    await Promise.resolve();
  };
  return { core, node, calls, jobs, advance, block: () => { options.blocked = 'enabled'; }, replace: () => { target = { ...node }; } };
}

test.each([false, true])('stable geometry is checked with frames=%s and all scheduled callbacks are cleaned', async frames => {
  const f = fixture({ frames });
  const result = f.core.__nomiCheckStates(f.node, ['visible', 'stable', 'enabled', 'editable']);
  await f.advance(100);
  expect(await result).toBeUndefined();
  expect(f.calls).toEqual([['visible', 'enabled', 'editable'], ['visible', 'enabled', 'editable']]);
  expect(f.jobs.size).toBe(0);
});

test('moving geometry is rejected, not retried until a convenient stable moment', async () => {
  const f = fixture({ moving: true });
  const result = f.core.__nomiCheckStates(f.node, ['visible', 'stable']);
  await f.advance(100);
  expect(await result).toEqual({ missingState: 'stable' });
  expect(f.jobs.size).toBe(0);
});

test.each(['detached', 'replaced'])('a %s node cannot satisfy a pending stability check', async mode => {
  const f = fixture();
  const result = f.core.__nomiCheckStates(f.node, ['visible', 'stable']);
  await f.advance(10);
  if (mode === 'detached') f.node.isConnected = false; else f.replace();
  await f.advance(100);
  expect(await result).toBe('error:notconnected');
  expect(f.jobs.size).toBe(0);
});

test('upstream actionability failure starts no polling', async () => {
  const f = fixture({ blocked: 'editable' });
  expect(await f.core.__nomiCheckStates(f.node, ['stable', 'editable'])).toEqual({ missingState: 'editable' });
  expect(f.jobs.size).toBe(0);
});

test('the deadline rejects a stalled scheduler and cancels pending callbacks', async () => {
  const f = fixture({ stalled: true });
  const result = f.core.__nomiCheckStates(f.node, ['visible', 'stable']);
  await f.advance(2000);
  expect(await result).toEqual({ missingState: 'stable' });
  expect(f.jobs.size).toBe(0);
});

test('enabled state is checked again after geometry settles', async () => {
  const f = fixture();
  const result = f.core.__nomiCheckStates(f.node, ['enabled', 'stable']);
  await f.advance(10);
  f.block();
  await f.advance(100);
  expect(await result).toEqual({ missingState: 'enabled' });
  expect(f.jobs.size).toBe(0);
});
