import { describe, expect, test } from 'bun:test';
import { EventEmitter } from 'node:events';
import { mkdtempSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { guardDevUpgradeSockets } from './vite-upgrade-sockets.mjs';

function fixture() {
  const server = new EventEmitter(); const socket = new EventEmitter(); let destroyed = false;
  socket.destroy = () => { destroyed = true; };
  const logs = []; const logger = { info: message => logs.push({ level: 'info', message }), error: (message, options) => logs.push({ level: 'error', message, error: options.error }) };
  return { server, socket, logs, logger, destroyed: () => destroyed };
}

describe('Vite development upgrade socket guard', () => {
  test('a bare peer reset closes only that connection and records the disconnect', () => {
    const item = fixture(); guardDevUpgradeSockets(item.server, item.logger);
    item.server.emit('upgrade', {}, item.socket);
    expect(() => item.socket.emit('error', Object.assign(new Error('read reset'), { code: 'ECONNRESET' }))).not.toThrow();
    expect(item.destroyed()).toBe(true); expect(item.logs).toHaveLength(1);
    expect(item.logs[0]).toMatchObject({ level: 'info' });
  });
  test('unexpected bare socket errors remain visible', () => {
    const item = fixture(); guardDevUpgradeSockets(item.server, item.logger);
    item.server.emit('upgrade', {}, item.socket);
    const error = Object.assign(new Error('transport failure'), { code: 'EIO' });
    item.socket.emit('error', error);
    expect(item.destroyed()).toBe(true); expect(item.logs).toHaveLength(1);
    expect(item.logs[0]).toMatchObject({ level: 'error', error });
    expect(item.logs[0].message).toContain('EIO');
    expect(item.logs[0].message).toContain('transport failure');
  });
  test('an existing websocket owner retains its error handler and cleanup', () => {
    const item = fixture(); let handled = 0; guardDevUpgradeSockets(item.server, item.logger);
    item.server.emit('upgrade', {}, item.socket);
    item.socket.on('error', () => { handled++; });
    item.socket.emit('error', Object.assign(new Error('owned'), { code: 'ECONNRESET' }));
    expect(handled).toBe(1); expect(item.logs).toEqual([]); expect(item.destroyed()).toBe(false);
  });
  test('installation is idempotent and server close removes its hook', () => {
    const item = fixture(); const dispose = guardDevUpgradeSockets(item.server, item.logger);
    expect(guardDevUpgradeSockets(item.server, item.logger)).toBe(dispose);
    expect(item.server.listenerCount('upgrade')).toBe(1);
    item.server.emit('close'); expect(item.server.listenerCount('upgrade')).toBe(0);
    expect(() => guardDevUpgradeSockets(null, item.logger)()).not.toThrow();
  });

  test.each(['isolated', 'project-config'])('actual Vite with %s configuration survives RST and retains HTTP and authenticated HMR', configuration => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-vite-upgrade-test-'));
    try {
      const child = spawnSync('node', [fileURLToPath(new URL('../test-fixtures/vite-upgrade-reset.mjs', import.meta.url)), 'guarded', root, configuration],
        { cwd: dirname(fileURLToPath(import.meta.url)), encoding: 'utf8', windowsHide: true, timeout: 8_000 });
      expect(child.status, child.stderr).toBe(0);
      const survived = child.stdout.split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line)).find(value => value.event === 'survived');
      expect(survived).toMatchObject({ httpStatus: 200, authenticatedHmrAccepted: true, globalExceptionListeners: 0 });
      expect(survived.projectConfig).toBe(configuration === 'project-config');
      if (configuration === 'project-config') expect(survived.projectGuardInstalled).toBe(true);
      expect(survived.logs.some(log => log.message.includes('ECONNRESET'))).toBe(true);
    } finally {
      const absolute = resolve(root);
      if (!absolute.startsWith(resolve(tmpdir()) + sep)) throw new Error('Fixture cleanup escaped temporary root');
      rmSync(absolute, { recursive: true, force: true });
    }
  }, 10_000);
});
