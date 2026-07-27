/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { BrowserViewerSocketState } from './browserViewerSocket';

type FakeMessageListener = (event: { data: unknown }) => void;

class FakeWebSocket {
  private readonly messageListeners: FakeMessageListener[] = [];

  addEventListener(type: 'message', listener: FakeMessageListener): void {
    if (type === 'message') this.messageListeners.push(listener);
  }

  receive(data: unknown): void {
    for (const listener of this.messageListeners) listener({ data });
  }
}

const createHarness = (initialControlState: 'agent' | 'user' | 'idle' = 'user') => {
  const socket = new FakeWebSocket();
  const connectionStates: string[] = [];
  const viewerErrors: Array<string | null> = [];
  const controlStates: string[] = [];
  const frameBindings: unknown[] = [];
  const frameSizes: unknown[] = [];
  const jpegFrames: ArrayBuffer[] = [];
  let inventoryRefreshes = 0;

  const state = new BrowserViewerSocketState({
    initialControlState,
    redactError: (message) => `redacted:${message}`,
    streamFailureMessage: () => 'stream failed',
    onConnectionState: (connectionState) => connectionStates.push(connectionState),
    onViewerError: (message) => viewerErrors.push(message),
    onFrameBinding: (binding) => frameBindings.push(binding),
    onFrameSize: (frame) => frameSizes.push(frame),
    onAddress: () => undefined,
    onActiveTabId: () => undefined,
    onControlState: (controlState) => controlStates.push(controlState),
    onInventoryRefresh: () => {
      inventoryRefreshes++;
    },
    onJpegFrame: (bytes) => jpegFrames.push(bytes),
  });
  socket.addEventListener('message', (event) => {
    state.received(event.data);
  });

  return {
    socket,
    state,
    connectionStates,
    viewerErrors,
    controlStates,
    frameBindings,
    frameSizes,
    jpegFrames,
    get inventoryRefreshes() {
      return inventoryRefreshes;
    },
  };
};

describe('browser viewer socket state', () => {
  test('recovers from a recoverable stream failure after valid frame metadata and JPEG', () => {
    const harness = createHarness();
    harness.state.opened();

    harness.socket.receive(
      JSON.stringify({
        type: 'viewer.metadata',
        frame: { width: 1280, height: 720 },
        frame_id: 'opaque-frame',
        frame_version: 42,
      })
    );
    const firstJpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xd9]).buffer;
    harness.socket.receive(firstJpeg);
    harness.socket.receive(
      JSON.stringify({
        type: 'stream_error',
        code: 'viewer_stream_failed',
        message: 'temporary backend failure',
        recoverable: true,
      })
    );

    expect(harness.state.snapshot.connectionState).toBe('failed');
    expect(harness.state.snapshot.error).toEqual({
      kind: 'stream',
      message: 'redacted:viewer_stream_failed: temporary backend failure',
      recoverable: true,
    });

    const recoveredJpeg = new Uint8Array([0xff, 0xd8, 1, 0xff, 0xd9]).buffer;
    harness.socket.receive(recoveredJpeg);

    expect(harness.state.snapshot.connectionState).toBe('streaming');
    expect(harness.state.snapshot.error).toBeNull();
    expect(harness.state.snapshot.frameBinding).toEqual({
      frame: { width: 1280, height: 720 },
      frame_id: 'opaque-frame',
      frame_version: 42,
    });
    expect(harness.connectionStates).toEqual(['streaming', 'failed', 'streaming']);
    expect(harness.viewerErrors).toEqual([
      'redacted:viewer_stream_failed: temporary backend failure',
      null,
    ]);
    expect(harness.frameSizes).toEqual([{ width: 1280, height: 720 }]);
    expect(harness.jpegFrames).toEqual([firstJpeg, recoveredJpeg]);
  });

  test('restores Agent control and refreshes inventory when the control lease expires', () => {
    const harness = createHarness('user');

    harness.socket.receive(
      JSON.stringify({
        type: 'command_error',
        code: 'control_lease_expired',
        message: 'lease expired',
        recoverable: true,
      })
    );

    expect(harness.state.snapshot.controlState).toBe('agent');
    expect(harness.state.snapshot.connectionState).toBe('connecting');
    expect(harness.controlStates).toEqual(['agent']);
    expect(harness.inventoryRefreshes).toBe(1);
  });

  test('echoes opaque frame identity on coordinate input without leaking target identity', () => {
    const harness = createHarness();

    harness.socket.receive(
      JSON.stringify({
        type: 'viewer.metadata',
        frame: {
          width: 800,
          height: 600,
          frameId: 'opaque/id:v1',
          frameSequence: 'version-token',
          target_id: 'backend-target-inside-frame',
        },
        target_id: 'backend-target-at-root',
        active_tab_id: 'tab-visible-to-ui',
      })
    );

    const bound = harness.state.bindInput({
      kind: 'pointer',
      action: 'down',
      x: 10,
      y: 20,
      target_id: 'caller-target',
      targetId: 'caller-target-camel',
      frame_id: 'caller-frame',
      frame_version: 999,
    });

    expect(bound).toEqual({
      kind: 'pointer',
      action: 'down',
      x: 10,
      y: 20,
      frame_id: 'opaque/id:v1',
      frame_version: 'version-token',
    });
    expect(JSON.stringify(bound).includes('target')).toBe(false);
    expect(harness.state.snapshot.frameBinding).toEqual({
      frame: { width: 800, height: 600 },
      frame_id: 'opaque/id:v1',
      frame_version: 'version-token',
    });
  });
});
