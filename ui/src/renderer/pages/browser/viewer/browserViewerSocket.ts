/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { BrowserLaneControlState } from '@/common/browser/browserTypes';
import {
  bindBrowserViewerInputToFrame,
  browserViewerErrorKind,
  parseBrowserViewerMetadata,
  transitionBrowserViewerJpegFrame,
  transitionBrowserViewerMetadata,
  type BrowserViewerFrameBinding,
  type BrowserViewerRuntimeState,
} from './browserViewerProtocol';

export interface BrowserViewerSocketStateDependencies {
  initialControlState: BrowserLaneControlState;
  redactError: (message: string) => string;
  streamFailureMessage: () => string;
  onConnectionState: (state: BrowserViewerRuntimeState['connectionState']) => void;
  onViewerError: (message: string | null) => void;
  onFrameBinding: (binding: BrowserViewerFrameBinding | null) => void;
  onFrameSize: (frame: BrowserViewerFrameBinding['frame']) => void;
  onAddress: (url: string) => void;
  onActiveTabId: (tabId: string) => void;
  onControlState: (state: BrowserLaneControlState) => void;
  onInventoryRefresh: () => void;
  onJpegFrame: (bytes: ArrayBuffer) => void;
}

/**
 * Keeps the WebSocket protocol state deterministic and separately testable
 * from React rendering. Binary JPEG bytes intentionally carry no target
 * identity; only optional validated opaque metadata can be echoed to input.
 */
export class BrowserViewerSocketState {
  private state: BrowserViewerRuntimeState;

  constructor(private readonly dependencies: BrowserViewerSocketStateDependencies) {
    this.state = {
      connectionState: 'connecting',
      error: null,
      controlState: dependencies.initialControlState,
      frameBinding: null,
    };
  }

  opened(): void {
    this.apply({
      ...this.state,
      connectionState: 'streaming',
    });
  }

  received(data: unknown): boolean {
    if (typeof data === 'string') {
      const metadata = parseBrowserViewerMetadata(data);
      if (!metadata) return false;
      const errorKind = browserViewerErrorKind(metadata);
      const errorMessage = errorKind
        ? this.dependencies.redactError(
            `${metadata.code ? `${metadata.code}: ` : ''}${
              metadata.message || this.dependencies.streamFailureMessage()
            }`
          )
        : undefined;
      const transition = transitionBrowserViewerMetadata(
        this.state,
        metadata,
        errorMessage
      );
      this.apply(transition.state);
      if (transition.acceptFrame && metadata.frame) this.dependencies.onFrameSize(metadata.frame);
      if (transition.acceptFrame && metadata.url) this.dependencies.onAddress(metadata.url);
      if (transition.acceptFrame && metadata.active_tab_id) {
        this.dependencies.onActiveTabId(metadata.active_tab_id);
      }
      if (transition.refreshInventory) this.dependencies.onInventoryRefresh();
      return true;
    }

    if (!(data instanceof ArrayBuffer) || data.byteLength === 0) return false;
    const transition = transitionBrowserViewerJpegFrame(this.state);
    this.apply(transition.state);
    if (transition.acceptFrame) this.dependencies.onJpegFrame(data);
    return true;
  }

  bindInput(input: Record<string, unknown>): Record<string, unknown> {
    return bindBrowserViewerInputToFrame(input, this.state.frameBinding);
  }

  get snapshot(): BrowserViewerRuntimeState {
    return {
      ...this.state,
      frameBinding: this.state.frameBinding
        ? {
            ...this.state.frameBinding,
            frame: { ...this.state.frameBinding.frame },
          }
        : null,
    };
  }

  private apply(next: BrowserViewerRuntimeState): void {
    const previous = this.state;
    this.state = next;
    if (previous.connectionState !== next.connectionState) {
      this.dependencies.onConnectionState(next.connectionState);
    }
    if (previous.error?.message !== next.error?.message) {
      this.dependencies.onViewerError(next.error?.message ?? null);
    }
    if (previous.controlState !== next.controlState) {
      this.dependencies.onControlState(next.controlState as BrowserLaneControlState);
    }
    if (previous.frameBinding !== next.frameBinding) {
      this.dependencies.onFrameBinding(next.frameBinding);
    }
  }
}
