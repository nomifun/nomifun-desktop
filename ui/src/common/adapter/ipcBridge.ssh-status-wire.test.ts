/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { InvalidEntityIdError } from '@/common/types/ids';
import { ssh, type IApiSshStatus } from './ipcBridge';

const source = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');
const SSH_HOST_ID = '0190f5fe-7c00-7a00-8000-0000000000a1';
const CONVERSATION_ID = '0190f5fe-7c00-7a00-8000-0000000000a2';
const realFetch = globalThis.fetch;

const rawStatus = (sshHostId: unknown) => ({
  sshHostId,
  conversationId: CONVERSATION_ID,
  state: 'reconnecting',
  attempt: 3,
  nextRetryInMs: 4_000,
  hostFingerprint: 'SHA256:abc',
  detail: 'transport closed',
  reaped: null,
  changedAt: 1,
});

function respondWith(data: unknown): void {
  globalThis.fetch = (() =>
    Promise.resolve(
      new Response(JSON.stringify({ success: true, data }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    )) as unknown as typeof fetch;
}

describe('ssh live-status wire contract', () => {
  test('the snapshot route and the push event name are the ones the backend serves', () => {
    // Plural on purpose: `/api/ssh-hosts/status` would be shadowed by the
    // `/{ssh_host_id}` capture on the same prefix.
    expect(source.includes("'/api/ssh-hosts/statuses'")).toBe(true);
    expect(source.includes("wsMappedEmitter<IApiSshStatus>('ssh.status'")).toBe(true);
  });

  test('every phase the backend can publish is a declared literal', () => {
    for (const phase of [
      'idle',
      'connecting',
      'connected',
      'degraded',
      'reconnecting',
      'dropped',
      'closed',
    ]) {
      expect(source.includes(`'${phase}'`)).toBe(true);
    }
    expect(source.includes('export type ISshLinkPhase')).toBe(true);
  });

  test('both the snapshot and the push path brand sshHostId through fromApiSshStatus', () => {
    expect(source.includes('const fromApiSshStatus')).toBe(true);
    expect(source.includes('sshHostId: parseSshHostId(value.sshHostId)')).toBe(true);
    // One mapper, two arrival paths — a status that came in over the socket must
    // not be shaped differently from one that came from the snapshot.
    expect(source.split('fromApiSshStatus').length - 1).toBeGreaterThanOrEqual(3);
  });

  test('the live status carries reaped so an unconfirmed exit is visible', () => {
    expect(source.includes('reaped: boolean | null')).toBe(true);
    expect(source.includes('nextRetryInMs: number | null')).toBe(true);
  });

  test('the live path never reads the host row status column', () => {
    // `IApiSshHost.status` is the host book's stale per-host hint (V9: it is
    // written once on connect and never walked back). Live link state comes
    // only from ssh.status / the statuses snapshot.
    expect(source.includes('IApiSshStatus')).toBe(true);
    const statusesBlock = source.slice(
      source.indexOf('const fromApiSshStatus'),
      source.indexOf('// -----', source.indexOf('onStatus'))
    );
    expect(statusesBlock.length).toBeGreaterThan(0);
    expect(statusesBlock.includes('value.status')).toBe(false);
    expect(statusesBlock.includes('host.status')).toBe(false);
  });

  test('snapshot rows are branded at the boundary and reject a legacy id', async () => {
    try {
      respondWith([rawStatus(SSH_HOST_ID)]);
      const rows: IApiSshStatus[] = await ssh.statuses.invoke();
      expect(rows[0]?.sshHostId).toBe(SSH_HOST_ID);
      expect(rows[0]?.state).toBe('reconnecting');
      expect(rows[0]?.nextRetryInMs).toBe(4_000);

      respondWith([rawStatus(`sshhost_${SSH_HOST_ID}`)]);
      let error: unknown;
      try {
        await ssh.statuses.invoke();
      } catch (caught) {
        error = caught;
      }
      expect(error instanceof InvalidEntityIdError).toBe(true);
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
