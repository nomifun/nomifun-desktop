/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IBrowserViewerToken } from '@/common/browser/browserTypes';
import { buildBrowserViewerUrl } from './browserViewerProtocol';

export interface BrowserViewerConnectionDependencies {
  mintViewerToken: (request: { lane_id: string }) => Promise<IBrowserViewerToken>;
  createSocket: (url: string) => WebSocket;
}

/**
 * Owns one viewer connection attempt at a time.
 *
 * A viewer token is a one-shot credential. It is deliberately not cached:
 * every connect/reconnect call mints a new token, and an invalidated pending
 * attempt is prevented from opening a socket after a newer attempt starts.
 */
export class BrowserViewerConnectionSession {
  private generation = 0;
  private socket: WebSocket | null = null;

  constructor(
    private readonly laneId: string,
    private readonly dependencies: BrowserViewerConnectionDependencies
  ) {}

  async connect(): Promise<WebSocket | null> {
    const generation = ++this.generation;
    this.closeSocket();

    const viewerToken = await this.dependencies.mintViewerToken({ lane_id: this.laneId });
    if (generation !== this.generation) return null;

    const socket = this.dependencies.createSocket(
      buildBrowserViewerUrl(this.laneId, viewerToken.token, viewerToken.view_url)
    );
    if (generation !== this.generation) {
      socket.close(1000, 'viewer attempt superseded');
      return null;
    }
    this.socket = socket;
    return socket;
  }

  reconnect(): Promise<WebSocket | null> {
    return this.connect();
  }

  close(): void {
    this.generation++;
    this.closeSocket();
  }

  private closeSocket(): void {
    const socket = this.socket;
    this.socket = null;
    socket?.close(1000, 'viewer changed');
  }
}
