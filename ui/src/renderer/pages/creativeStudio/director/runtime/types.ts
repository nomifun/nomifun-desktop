/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { DirectorCaptureRequest, DirectorState } from '../domain';

/**
 * The only asset boundary exposed by the renderer. The host resolves a stable
 * NomiFun asset ID to a URL it trusts; the 3D runtime never accepts paths or
 * opaque asset DTOs from project documents.
 */
export type DirectorAssetUrlResolver = (
  assetId: string,
  signal: AbortSignal
) => string | null | Promise<string | null>;

export type DirectorRuntimeErrorCode =
  | 'asset-url'
  | 'asset-fetch'
  | 'asset-decode'
  | 'capture'
  | 'renderer';

export interface DirectorRuntimeError {
  code: DirectorRuntimeErrorCode;
  message: string;
  assetId?: string;
  cause?: unknown;
}

export interface DirectorRuntimeOptions {
  container: HTMLElement;
  resolveAssetUrl: DirectorAssetUrlResolver;
  /** Caps GPU allocation on high-density displays. Defaults to 2. */
  maxPixelRatio?: number;
  /** Real editor helper; no geometry is substituted for missing assets. */
  showAxes?: boolean;
  onError?(error: DirectorRuntimeError): void;
}

export type DirectorImageCaptureRequest = Extract<
  DirectorCaptureRequest,
  { kind: 'image' }
>;

export interface DirectorImageCaptureResult {
  requestId: string;
  cameraId: string;
  width: number;
  height: number;
  format: 'png' | 'jpeg';
  blob: Blob;
}

export interface DirectorRuntimeHandle {
  readonly canvas: HTMLCanvasElement;
  update(state: DirectorState, timeSeconds?: number): void;
  resize(): void;
  start(): void;
  stop(): void;
  captureImage(request: DirectorImageCaptureRequest): Promise<DirectorImageCaptureResult>;
  dispose(): void;
}
