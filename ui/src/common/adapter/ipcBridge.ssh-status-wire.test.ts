/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { InvalidEntityIdError, parseSshHostId } from '@/common/types/ids';
import { agentPlatform, ssh, type IApiSshStatus } from './ipcBridge';

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
  retryable: null,
  changedAt: 1,
});

const rawTerminalDrop = () => ({
  ...rawStatus(SSH_HOST_ID),
  state: 'dropped',
  attempt: 0,
  nextRetryInMs: null,
  detail: 'ssh authentication failed: rejected',
  retryable: false,
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
  test('connection tests post the draft, with an optional owned host for unchanged secrets', async () => {
    const requests: { url: string; method?: string; body: unknown }[] = [];
    globalThis.fetch = (async (input, init) => {
      requests.push({ url: String(input), method: init?.method, body: JSON.parse(String(init?.body)) });
      return new Response(JSON.stringify({ success: true, data: { ok: true, message: 'connected' } }), {
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;
    try {
      const draft = { host: 'draft.example', port: 2222, username: 'deploy', authType: 'password' as const, password: 'draft-secret' };
      expect(await ssh.testConnection.invoke(draft)).toEqual({ ok: true, message: 'connected' });
      await ssh.testConnection.invoke({ ...draft, password: undefined, sshHostId: parseSshHostId(SSH_HOST_ID) });
      expect(requests.map(({ url }) => new URL(url, 'http://127.0.0.1').pathname)).toEqual([
        '/api/ssh-hosts/test-connection', '/api/ssh-hosts/test-connection',
      ]);
      expect(requests.every(({ method }) => method === 'POST')).toBe(true);
      expect(requests[0]!.body).toEqual(draft);
      expect(requests[1]!.body).toEqual({ host: draft.host, port: draft.port, username: draft.username, authType: draft.authType, sshHostId: SSH_HOST_ID });
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('link controls post only the canonical session identity in the URL', async () => {
    const requests: { url: string; method?: string; body: BodyInit | null | undefined }[] = [];
    globalThis.fetch = (async (input, init) => {
      requests.push({ url: String(input), method: init?.method, body: init?.body });
      return new Response(JSON.stringify({ success: true, data: { accepted: true } }), {
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;
    try {
      expect(await agentPlatform.sessions.connectSsh.invoke({ agent_session_id: CONVERSATION_ID })).toEqual({ accepted: true });
      expect(await agentPlatform.sessions.disconnectSsh.invoke({ agent_session_id: CONVERSATION_ID })).toEqual({ accepted: true });
      expect(requests.map((request) => new URL(request.url, 'http://127.0.0.1').pathname)).toEqual([
        `/api/agent-sessions/${CONVERSATION_ID}/ssh/connect`,
        `/api/agent-sessions/${CONVERSATION_ID}/ssh/disconnect`,
      ]);
      expect(requests.every((request) => request.method === 'POST' && request.body == null)).toBe(true);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

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

  test('the live status carries retryable so a terminal drop can ask for action', () => {
    // The backend's `SshLinkState::Dropped` knows whether a retry could help;
    // without that bit on the wire the client can only string-match `detail`,
    // which is exactly how "authentication failed" gets shown as a blip that
    // will clear itself.
    expect(source.includes('retryable: boolean | null')).toBe(true);
  });

  test('a terminal drop survives the snapshot mapper with retryable intact', async () => {
    try {
      respondWith([rawTerminalDrop()]);
      const rows: IApiSshStatus[] = await ssh.statuses.invoke();
      expect(rows[0]?.state).toBe('dropped');
      expect(rows[0]?.retryable).toBe(false);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('live link status is independent of the saved host profile', () => {
    expect(source.includes('IApiSshStatus')).toBe(true);
    const mapperStart = source.indexOf('const fromApiSshStatus');
    const statusesBlock = source.slice(
      mapperStart,
      source.indexOf('// -----', source.indexOf('onStatus:', mapperStart))
    );
    expect(statusesBlock.length).toBeGreaterThan(0);
    expect(statusesBlock.includes('value.status')).toBe(false);
    expect(statusesBlock.includes('host.status')).toBe(false);
    const hostDto = source.slice(source.indexOf('export interface IApiSshHost {'), source.indexOf('/** Create payload'));
    expect(hostDto.includes('status:')).toBe(false);
    expect(hostDto.includes('lastConnectedAt')).toBe(false);
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
