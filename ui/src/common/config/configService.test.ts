import { describe, expect, test } from 'bun:test';
import { ConfigServiceImpl } from './configService';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function fixture() {
  const calls: Array<{
    method: string;
    path: string;
    body: unknown;
    response: ReturnType<typeof deferred<unknown>>;
  }> = [];
  // Each test owns its service and transport; no singleton, fetch or module mocks.
  const service = new ConfigServiceImpl(<T>(method: string, path: string, body?: unknown) => {
    const response = deferred<unknown>();
    calls.push({ method, path, body, response });
    return response.promise as Promise<T>;
  });
  async function load(reload = false) {
    const done = reload ? service.reload() : service.initialize();
    await Promise.resolve();
    return { done, response: calls.at(-1)!.response };
  }
  return { service, calls, load };
}

type Mutation = 'set' | 'setLocal' | 'remove' | 'setBatch';
const mutations: Mutation[] = ['set', 'setLocal', 'remove', 'setBatch'];
function mutate(service: ConfigServiceImpl, operation: Mutation): Promise<void> {
  switch (operation) {
    case 'set': return service.set('theme', 'dark');
    case 'setLocal': service.setLocal('theme', 'dark'); return Promise.resolve();
    case 'remove': return service.remove('theme');
    case 'setBatch': return service.setBatch({ theme: 'dark', language: 'zh-CN' });
  }
}

describe('configService load ownership', () => {
  test('initialize and whenReady share one request, including after success', async () => {
    const { service, calls, load } = fixture();
    const first = await load();
    expect(service.initialize()).toBe(first.done);
    expect(service.whenReady()).toBe(first.done);
    expect(calls).toHaveLength(1);
    first.response.resolve({ theme: 'dark' });
    await first.done;
    expect(service.isInitialized()).toBe(true);
    expect(service.get('theme')).toBe('dark');
    expect(service.initialize()).toBe(first.done);
    expect(calls).toHaveLength(1);
  });

  test('failed loading resolves, clears old cache, and permits retry', async () => {
    const { service, load } = fixture();
    service.setLocal('theme', 'old');
    const first = await load();
    first.response.reject(new Error('test offline'));
    await first.done;
    expect(service.isInitialized()).toBe(false);
    expect(service.get('theme')).toBeUndefined();
    const retry = await load();
    expect(retry.done).not.toBe(first.done);
    retry.response.resolve({ theme: 'dark' });
    await retry.done;
    expect(service.get('theme')).toBe('dark');
  });

  test('a synchronously throwing transport still resolves and permits retry', async () => {
    let attempts = 0;
    const service = new ConfigServiceImpl(<T>() => {
      attempts++;
      if (attempts === 1) throw new Error('test synchronous failure');
      return Promise.resolve({ theme: 'recovered' } as T);
    });
    await service.initialize();
    expect(service.isInitialized()).toBe(false);
    await service.initialize();
    expect(attempts).toBe(2);
    expect(service.get('theme')).toBe('recovered');
  });

  test('reset before dispatch skips the invalidated GET', async () => {
    const { service, calls } = fixture();
    const loading = service.initialize();
    service.reset();
    await loading;
    expect(calls).toHaveLength(0);
    expect(service.isInitialized()).toBe(false);
  });

  for (const outcome of ['success', 'failure'] as const) {
    for (const newerComplete of [false, true]) {
      test('ignores stale ' + outcome + ' with newer load ' + (newerComplete ? 'complete' : 'pending'), async () => {
        const { service, calls, load } = fixture();
        const old = await load();
        const current = await load(true);
        const currentPromise = service.whenReady();
        if (newerComplete) {
          current.response.resolve({ theme: 'current' });
          await current.done;
        }
        if (outcome === 'success') old.response.resolve({ theme: 'stale' });
        else old.response.reject(new Error('test stale failure'));
        await old.done;
        expect(service.get('theme')).toBe(newerComplete ? 'current' : undefined);
        expect(service.isInitialized()).toBe(newerComplete);
        expect(service.whenReady()).toBe(currentPromise);
        expect(calls).toHaveLength(2);
        if (!newerComplete) {
          current.response.resolve({ theme: 'current' });
          await current.done;
          expect(service.get('theme')).toBe('current');
        }
      });
    }

    test('reset invalidates an in-flight ' + outcome, async () => {
      const { service, load } = fixture();
      const old = await load();
      let notifications = 0;
      service.subscribe('theme', () => notifications++);
      service.reset();
      service.setLocal('theme', 'after-reset');
      if (outcome === 'success') old.response.resolve({ theme: 'stale' });
      else old.response.reject(new Error('test stale failure'));
      await old.done;
      expect(service.get('theme')).toBe('after-reset');
      expect(service.isInitialized()).toBe(false);
      expect(notifications).toBe(0);
    });
  }

  test('a load publishes a coherent snapshot and notifies changes and deletions', async () => {
    const { service, load } = fixture();
    service.setLocal('theme', 'old');
    service.setLocal('language', 'en-US');
    service.setLocal('colorScheme', 'unchanged');
    const seen: unknown[] = [];
    service.subscribe('theme', (value) => seen.push([value, service.get('language'), service.isInitialized()]));
    service.subscribe('language', (value) => seen.push(['language', value]));
    service.subscribe('colorScheme', () => seen.push('unexpected'));
    const first = await load();
    first.response.resolve({ theme: 'dark', colorScheme: 'unchanged' });
    await first.done;
    expect(seen).toEqual([['dark', undefined, true], ['language', undefined]]);
  });

  test('failed reload notifies subscribers that persisted values were cleared', async () => {
    const { service, load } = fixture();
    const first = await load();
    first.response.resolve({ theme: 'dark' });
    await first.done;
    const seen: unknown[] = [];
    service.subscribe('theme', (value) => seen.push([value, service.isInitialized()]));
    const next = await load(true);
    next.response.reject(new Error('test offline'));
    await next.done;
    expect(seen).toEqual([[undefined, false]]);
  });
});

describe('configService optimistic cache', () => {
  for (const operation of mutations) {
    for (const outcome of ['success', 'failure'] as const) {
      test(operation + ' survives overlapping GET ' + outcome, async () => {
        const { service, calls, load } = fixture();
        service.setLocal('theme', 'old');
        const loading = await load();
        const writing = mutate(service, operation);
        if (operation !== 'setLocal') calls.at(-1)!.response.resolve(undefined);
        await writing;
        if (outcome === 'success') loading.response.resolve({ theme: 'stale', language: 'en-US', colorScheme: 'blue' });
        else loading.response.reject(new Error('test offline'));
        await loading.done;
        expect(service.get('theme')).toBe(operation === 'remove' ? undefined : 'dark');
        if (operation === 'setBatch') expect(service.get('language')).toBe('zh-CN');
        expect(service.get('colorScheme')).toBe(outcome === 'success' ? 'blue' : undefined);
      });
    }
  }

  for (const operation of ['set', 'remove', 'setBatch'] as const) {
    test('reload protects ' + operation + ' already pending at GET start', async () => {
      const { service, calls, load } = fixture();
      const writing = mutate(service, operation);
      const put = calls[0]!;
      expect(put.method).toBe('PUT');
      expect(put.path).toBe('/api/settings/client');
      if (operation === 'remove') expect(put.body).toEqual({ theme: null });
      const loading = await load(true);
      put.response.resolve(undefined);
      await writing;
      loading.response.resolve({ theme: 'stale', language: 'en-US' });
      await loading.done;
      expect(service.get('theme')).toBe(operation === 'remove' ? undefined : 'dark');
      if (operation === 'setBatch') expect(service.get('language')).toBe('zh-CN');
      // Protection is per load, not a permanent override of server authority.
      const next = await load(true);
      next.response.resolve({ theme: 'server' });
      await next.done;
      expect(service.get('theme')).toBe('server');
    });
  }

  test('finishing one PUT does not unprotect another pending PUT of the same key', async () => {
    const { service, calls, load } = fixture();
    const first = service.set('theme', 'first');
    const second = service.set('theme', 'second');
    calls[0]!.response.resolve(undefined);
    await first;
    const loading = await load(true);
    loading.response.resolve({ theme: 'first' });
    await loading.done;
    expect(service.get('theme')).toBe('second');
    calls[1]!.response.resolve(undefined);
    await second;
  });

  test('a failed PUT still rejects and lets the caller reload persisted state', async () => {
    const { service, calls, load } = fixture();
    const failure = new Error('test write rejected');
    const writing = service.set('theme', 'optimistic').catch((error: unknown) => error);
    calls[0]!.response.reject(failure);
    expect(await writing).toBe(failure);
    expect(service.get('theme')).toBe('optimistic');
    const reload = await load(true);
    reload.response.resolve({ theme: 'persisted' });
    await reload.done;
    expect(service.get('theme')).toBe('persisted');
  });

  test('reset forgets old pending writes without their completion affecting new writes', async () => {
    const { service, calls, load } = fixture();
    const old = service.set('theme', 'old');
    service.reset();
    const afterReset = await load();
    afterReset.response.resolve({ theme: 'server' });
    await afterReset.done;
    expect(service.get('theme')).toBe('server');
    const current = service.set('theme', 'current');
    const currentPut = calls.at(-1)!;
    calls[0]!.response.resolve(undefined);
    await old;
    const reloading = await load(true);
    reloading.response.resolve({ theme: 'stale' });
    await reloading.done;
    expect(service.get('theme')).toBe('current');
    currentPut.response.resolve(undefined);
    await current;
  });
});

describe('configService subscribers', () => {
  test('a throwing load subscriber cannot clear the loaded snapshot or stop notifications', async () => {
    const { service, load } = fixture();
    service.subscribe('theme', () => { throw new Error('test load subscriber failure'); });
    const seen: unknown[] = [];
    service.subscribe('theme', (value) => seen.push(value));
    service.subscribe('language', (value) => seen.push(value));
    const loading = await load();
    loading.response.resolve({ theme: 'dark', language: 'zh-CN' });
    await loading.done;
    expect(service.isInitialized()).toBe(true);
    expect(service.get('theme')).toBe('dark');
    expect(seen).toEqual(['dark', 'zh-CN']);
    expect(service.whenReady()).toBe(loading.done);
  });

  for (const operation of mutations) {
    test('a throwing subscriber cannot interrupt ' + operation, async () => {
      const { service, calls } = fixture();
      service.subscribe('theme', () => { throw new Error('test subscriber failure'); });
      const seen: unknown[] = [];
      service.subscribe('theme', (value) => seen.push(value));
      service.subscribe('language', (value) => seen.push(value));
      // Capture rejection immediately so the old, broken implementation has no
      // unhandled rejection even when an assertion fails before awaiting it.
      const writing = Promise.resolve().then(() => mutate(service, operation)).catch((error: unknown) => error);
      await Promise.resolve();
      expect(seen).toEqual(operation === 'setBatch' ? ['dark', 'zh-CN'] : [operation === 'remove' ? undefined : 'dark']);
      expect(calls).toHaveLength(operation === 'setLocal' ? 0 : 1);
      if (operation !== 'setLocal') {
        expect(calls[0]!.method).toBe('PUT');
        calls[0]!.response.resolve(undefined);
      }
      expect(await writing).toBeUndefined();
    });
  }

  test('batch subscribers see all entries already updated', async () => {
    const { service, calls } = fixture();
    service.setLocal('language', 'en-US');
    const seen: unknown[] = [];
    service.subscribe('theme', () => seen.push(service.get('language')));
    const writing = service.setBatch({ theme: 'dark', language: 'zh-CN' });
    expect(seen).toEqual(['zh-CN']);
    expect(calls[0]!.body).toEqual({ theme: 'dark', language: 'zh-CN' });
    calls[0]!.response.resolve(undefined);
    await writing;
  });

  test('old unsubscribe cannot detach the same callback registered after reset', () => {
    const { service } = fixture();
    const seen: unknown[] = [];
    const callback = (value: unknown) => { seen.push(value); };
    const oldUnsubscribe = service.subscribe('theme', callback);
    service.reset();
    const currentUnsubscribe = service.subscribe('theme', callback);
    oldUnsubscribe();
    service.setLocal('theme', 'current');
    currentUnsubscribe();
    service.setLocal('theme', 'not-observed');
    expect(seen).toEqual(['current']);
  });
});
