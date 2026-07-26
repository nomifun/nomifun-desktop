/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { BrowserViewerConnectionSession } from './browserViewerConnection';

class FakeSocket {
  readonly closes: Array<{ code?: number; reason?: string }> = [];

  constructor(readonly url: string) {}

  close(code?: number, reason?: string): void {
    this.closes.push({ code, reason });
  }
}

const asWebSocket = (socket: FakeSocket): WebSocket => socket as unknown as WebSocket;

describe('browser viewer connection session', () => {
  test('mints a distinct one-shot token for initial connect and every reconnect', async () => {
    const mintedTokens = ['a'.repeat(64), 'b'.repeat(64), 'c'.repeat(64)];
    const mintRequests: string[] = [];
    const sockets: FakeSocket[] = [];
    const session = new BrowserViewerConnectionSession('lane/one', {
      mintViewerToken: async ({ lane_id }) => {
        mintRequests.push(lane_id);
        const token = mintedTokens[mintRequests.length - 1];
        if (!token) throw new Error('unexpected token request');
        return {
          token,
          view_url: `/api/browser/lanes/lane%2Fone/view?token=${'f'.repeat(64)}`,
        };
      },
      createSocket: (url) => {
        const socket = new FakeSocket(url);
        sockets.push(socket);
        return asWebSocket(socket);
      },
    });

    await session.connect();
    await session.reconnect();
    await session.reconnect();

    expect(mintRequests).toEqual(['lane/one', 'lane/one', 'lane/one']);
    expect(sockets.map((socket) => new URL(socket.url, 'https://example.test').searchParams.get('token')))
      .toEqual(mintedTokens);
    expect(sockets[0]?.closes).toEqual([{ code: 1000, reason: 'viewer changed' }]);
    expect(sockets[1]?.closes).toEqual([{ code: 1000, reason: 'viewer changed' }]);
    expect(sockets[2]?.closes).toEqual([]);
  });

  test('does not open a socket with a token minted for a superseded attempt', async () => {
    let resolveFirst:
      | ((value: { token: string; view_url: string }) => void)
      | undefined;
    const sockets: FakeSocket[] = [];
    let mintCount = 0;
    const session = new BrowserViewerConnectionSession('lane-race', {
      mintViewerToken: () => {
        mintCount++;
        if (mintCount === 1) {
          return new Promise((resolve) => {
            resolveFirst = resolve;
          });
        }
        return Promise.resolve({
          token: '2'.repeat(64),
          view_url: '/api/browser/lanes/lane-race/view',
        });
      },
      createSocket: (url) => {
        const socket = new FakeSocket(url);
        sockets.push(socket);
        return asWebSocket(socket);
      },
    });

    const firstAttempt = session.connect();
    const secondAttempt = session.reconnect();
    await secondAttempt;
    resolveFirst?.({
      token: '1'.repeat(64),
      view_url: '/api/browser/lanes/lane-race/view',
    });

    expect(await firstAttempt).toBeNull();
    expect(mintCount).toBe(2);
    expect(sockets).toHaveLength(1);
    expect(new URL(sockets[0]!.url, 'https://example.test').searchParams.get('token')).toBe(
      '2'.repeat(64)
    );
  });

  test('invalidates a pending token mint when the viewer is closed', async () => {
    let resolveMint:
      | ((value: { token: string; view_url: string }) => void)
      | undefined;
    let socketCount = 0;
    const session = new BrowserViewerConnectionSession('lane-close', {
      mintViewerToken: () =>
        new Promise((resolve) => {
          resolveMint = resolve;
        }),
      createSocket: () => {
        socketCount++;
        return asWebSocket(new FakeSocket('unused'));
      },
    });

    const attempt = session.connect();
    session.close();
    resolveMint?.({
      token: '3'.repeat(64),
      view_url: '/api/browser/lanes/lane-close/view',
    });

    expect(await attempt).toBeNull();
    expect(socketCount).toBe(0);
  });
});
