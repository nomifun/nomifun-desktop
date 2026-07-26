/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export interface BrowserFrameSize {
  width: number;
  height: number;
}

export interface BrowserViewerMetadata {
  type: string;
  frame?: BrowserFrameSize;
  /**
   * Optional opaque identity for the JPEG represented by this metadata.
   * The renderer may echo it with pointer/wheel input, but must never derive
   * or expose the backend's raw browser target id.
   */
  frame_id?: string;
  frame_version?: string | number;
  title?: string;
  url?: string;
  active_tab_id?: string;
  control_state?: string;
  message?: string;
  code?: string;
  recoverable?: boolean;
}

export type BrowserViewerConnectionState = 'idle' | 'connecting' | 'streaming' | 'failed';
export type BrowserViewerErrorKind = 'stream' | 'command' | 'protocol' | null;
export type BrowserViewerOpaqueFrameVersion = string | number;

export interface BrowserViewerFrameBinding {
  frame: BrowserFrameSize;
  frame_id?: string;
  frame_version?: BrowserViewerOpaqueFrameVersion;
}

export interface BrowserViewerErrorState {
  kind: Exclude<BrowserViewerErrorKind, null>;
  message: string;
  recoverable: boolean;
}

export interface BrowserViewerRuntimeState {
  connectionState: BrowserViewerConnectionState;
  error: BrowserViewerErrorState | null;
  controlState: string;
  frameBinding: BrowserViewerFrameBinding | null;
}

export interface BrowserViewerStateTransition {
  state: BrowserViewerRuntimeState;
  refreshInventory: boolean;
  acceptFrame: boolean;
}

export const browserViewerErrorKind = (
  metadata: Pick<BrowserViewerMetadata, 'type' | 'code'>
): BrowserViewerErrorKind => {
  if (metadata.type === 'command_error') return 'command';
  if (metadata.type === 'protocol_error') return 'protocol';
  if (metadata.type === 'stream_error' || metadata.code === 'viewer_stream_failed') {
    return 'stream';
  }
  return null;
};

export const isBrowserViewerStreamFailure = (
  metadata: Pick<BrowserViewerMetadata, 'type' | 'code'>
): boolean => browserViewerErrorKind(metadata) === 'stream';

export const browserViewerStateAfterMetadata = (
  current: BrowserViewerConnectionState,
  metadata: Pick<BrowserViewerMetadata, 'type' | 'code'>
): BrowserViewerConnectionState =>
  browserViewerErrorKind(metadata) === 'stream' ? 'failed' : current;

const CONTROL_RETURNED_MESSAGE = 'browser control was returned to the agent';
const CONTROL_LEASE_INVALID_MESSAGE =
  'browser control lease is missing, expired, or no longer current';
const OPAQUE_FRAME_TOKEN_MAX_LENGTH = 256;

const isSafeOpaqueFrameToken = (value: string): boolean =>
  value.length > 0 &&
  value.length <= OPAQUE_FRAME_TOKEN_MAX_LENGTH &&
  !/[\u0000-\u001f\u007f]/u.test(value);

const asOpaqueFrameVersion = (
  value: unknown
): BrowserViewerOpaqueFrameVersion | undefined => {
  if (typeof value === 'string') {
    return isSafeOpaqueFrameToken(value) ? value : undefined;
  }
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
    ? value
    : undefined;
};

const validatedOpaqueFrameId = (value: unknown): string | undefined =>
  typeof value === 'string' && isSafeOpaqueFrameToken(value) ? value : undefined;

const validatedOpaqueFrameVersion = (
  value: unknown
): BrowserViewerOpaqueFrameVersion | undefined => asOpaqueFrameVersion(value);

const hasOwn = (value: Record<string, unknown>, key: string): boolean =>
  Object.prototype.hasOwnProperty.call(value, key);

const firstOwnValue = (
  value: Record<string, unknown>,
  keys: readonly string[]
): unknown => {
  for (const key of keys) {
    if (hasOwn(value, key)) return value[key];
  }
  return undefined;
};

/**
 * Current servers encode a returned/revoked viewer lease as a recoverable
 * operation_not_allowed command error with this stable safe message. Future
 * servers may use a dedicated structured code; support those without treating
 * every operation_not_allowed as a control transition.
 */
export const isBrowserViewerControlInvalidation = (
  metadata: Pick<BrowserViewerMetadata, 'type' | 'code' | 'message'>
): boolean => {
  if (metadata.type !== 'command_error') return false;
  const code = metadata.code?.trim().toLowerCase();
  if (
    code === 'control_lease_expired' ||
    code === 'control_lease_invalid' ||
    code === 'control_lease_revoked' ||
    code === 'control_lease_replaced' ||
    code === 'viewer_control_returned' ||
    code === 'viewer_control_revoked' ||
    code === 'viewer_control_expired'
  ) {
    return true;
  }
  return (
    code === 'operation_not_allowed' &&
    (metadata.message?.trim().toLowerCase().includes(CONTROL_RETURNED_MESSAGE) === true ||
      metadata.message
        ?.trim()
        .toLowerCase()
        .includes(CONTROL_LEASE_INVALID_MESSAGE) === true)
  );
};

export const browserViewerFrameBindingFromMetadata = (
  metadata: BrowserViewerMetadata
): BrowserViewerFrameBinding | null =>
  metadata.frame
    ? {
        frame: metadata.frame,
        ...(validatedOpaqueFrameId(metadata.frame_id)
          ? { frame_id: metadata.frame_id }
          : {}),
        ...(validatedOpaqueFrameVersion(metadata.frame_version) !== undefined
          ? { frame_version: metadata.frame_version }
          : {}),
      }
    : null;

/**
 * Echoes only protocol-declared opaque frame identity onto coordinate input.
 * No fallback to target/tab/url state is allowed.
 */
export const bindBrowserViewerInputToFrame = (
  input: Record<string, unknown>,
  binding: BrowserViewerFrameBinding | null
): Record<string, unknown> => {
  const sanitized = { ...input };
  for (const key of [
    'target_id',
    'targetId',
    'frame_id',
    'frameId',
    'opaque_frame_id',
    'opaqueFrameId',
    'frame_version',
    'frameVersion',
    'frame_sequence',
    'frameSequence',
    'frame_seq',
    'frameSeq',
  ]) {
    delete sanitized[key];
  }
  if (!binding || (input.kind !== 'pointer' && input.kind !== 'wheel')) {
    return sanitized;
  }
  const frameId = validatedOpaqueFrameId(binding.frame_id);
  const frameVersion = validatedOpaqueFrameVersion(binding.frame_version);
  return {
    ...sanitized,
    ...(frameId ? { frame_id: frameId } : {}),
    ...(frameVersion !== undefined ? { frame_version: frameVersion } : {}),
  };
};

export const transitionBrowserViewerMetadata = (
  current: BrowserViewerRuntimeState,
  metadata: BrowserViewerMetadata,
  errorMessage?: string
): BrowserViewerStateTransition => {
  const errorKind = browserViewerErrorKind(metadata);
  const controlInvalidated = isBrowserViewerControlInvalidation(metadata);
  const terminalStreamFailure =
    current.error?.kind === 'stream' && !current.error.recoverable;
  const acceptFrame = !terminalStreamFailure;
  const nextFrameBinding =
    browserViewerFrameBindingFromMetadata(metadata) ?? current.frameBinding;
  const clearRecoverableStreamError =
    Boolean(metadata.frame) &&
    current.error?.kind === 'stream' &&
    current.error.recoverable;

  return {
    state: {
      connectionState:
        errorKind === 'stream'
          ? 'failed'
          : acceptFrame && metadata.frame
            ? 'streaming'
            : current.connectionState,
      error: errorKind
        ? {
            kind: errorKind,
            message: errorMessage ?? metadata.message ?? metadata.code ?? errorKind,
            recoverable: metadata.recoverable === true,
          }
        : clearRecoverableStreamError
          ? null
          : current.error,
      controlState: controlInvalidated
        ? 'agent'
        : metadata.control_state ?? current.controlState,
      frameBinding: acceptFrame ? nextFrameBinding : current.frameBinding,
    },
    refreshInventory: controlInvalidated,
    acceptFrame,
  };
};

export const transitionBrowserViewerJpegFrame = (
  current: BrowserViewerRuntimeState
): BrowserViewerStateTransition => {
  const terminalStreamFailure =
    current.error?.kind === 'stream' && !current.error.recoverable;
  if (terminalStreamFailure) {
    return {
      state: current,
      refreshInventory: false,
      acceptFrame: false,
    };
  }

  return {
    state: {
      ...current,
      connectionState: 'streaming',
      error:
        current.error?.kind === 'stream' && current.error.recoverable
          ? null
          : current.error,
    },
    refreshInventory: false,
    acceptFrame: true,
  };
};

export interface BrowserViewerPoint {
  x: number;
  y: number;
}

const asRecord = (value: unknown): Record<string, unknown> =>
  value != null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};

const positiveNumber = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : undefined;

export const parseBrowserViewerMetadata = (raw: string): BrowserViewerMetadata | null => {
  try {
    const value = asRecord(JSON.parse(raw));
    const payload = asRecord(value.data ?? value.payload);
    const source = Object.keys(payload).length > 0 ? { ...value, ...payload } : value;
    const type =
      (typeof source.type === 'string' && source.type) ||
      (typeof source.event === 'string' && source.event) ||
      (typeof source.name === 'string' && source.name);
    if (!type) return null;
    const rawFrame = asRecord(source.frame ?? source.viewport ?? source.dimensions);
    const frameSource = Object.keys(rawFrame).length > 0
      ? { ...source, ...rawFrame }
      : source;
    const width =
      positiveNumber(rawFrame.width) ??
      positiveNumber(source.width) ??
      positiveNumber(source.frame_width);
    const height =
      positiveNumber(rawFrame.height) ??
      positiveNumber(source.height) ??
      positiveNumber(source.frame_height);
    const rawFrameId = firstOwnValue(frameSource, [
      'frame_id',
      'frameId',
      'opaque_frame_id',
      'opaqueFrameId',
    ]);
    const frameId =
      typeof rawFrameId === 'string' && isSafeOpaqueFrameToken(rawFrameId)
        ? rawFrameId
        : undefined;
    const frameVersion = asOpaqueFrameVersion(
      firstOwnValue(frameSource, [
        'frame_version',
        'frameVersion',
        'frame_sequence',
        'frameSequence',
        'frame_seq',
        'frameSeq',
      ])
    );

    return {
      type,
      frame: width && height ? { width, height } : undefined,
      frame_id: frameId,
      frame_version: frameVersion,
      title: typeof source.title === 'string' ? source.title : undefined,
      url: typeof source.url === 'string' ? source.url : undefined,
      active_tab_id:
        typeof source.active_tab_id === 'string'
          ? source.active_tab_id
          : typeof source.tab_id === 'string'
            ? source.tab_id
            : undefined,
      control_state:
        typeof source.control_state === 'string'
          ? source.control_state
          : typeof source.control === 'string'
            ? source.control
            : undefined,
      message: typeof source.message === 'string' ? source.message : undefined,
      code: typeof source.code === 'string' ? source.code : undefined,
      recoverable:
        typeof source.recoverable === 'boolean'
          ? source.recoverable
          : typeof source.retryable === 'boolean'
            ? source.retryable
            : undefined,
    };
  } catch {
    return null;
  }
};

export const buildBrowserViewerUrl = (
  laneId: string,
  token: string,
  serverUrl?: string | null
): string => {
  const fallback = `/api/browser/lanes/${encodeURIComponent(laneId)}/view`;
  const input = serverUrl?.trim() || fallback;
  let parsed: URL;
  try {
    parsed = new URL(input, 'http://nomifun.invalid/');
  } catch {
    throw new TypeError('Browser viewer URL is invalid');
  }
  if (parsed.pathname !== fallback) {
    throw new TypeError('Browser viewer URL must target the selected lane');
  }
  if (parsed.username || parsed.password || parsed.hash) {
    throw new TypeError('Browser viewer URL must not contain credentials or a fragment');
  }

  // The response's freshly minted token is authoritative. Never retain a
  // pre-populated token from a server URL: it may be stale, consumed, or scoped
  // to a different lane.
  parsed.searchParams.set('token', token);

  const isAbsolute = /^[a-z][a-z0-9+.-]*:\/\//i.test(input) || input.startsWith('//');
  return isAbsolute ? parsed.toString() : `${parsed.pathname}${parsed.search}`;
};

/**
 * Maps a pointer on an object-contain image to the exact encoded frame.
 * Events in the letterbox are rejected instead of being clamped onto content.
 */
export const mapBrowserViewerPoint = (
  rect: Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>,
  frame: BrowserFrameSize,
  clientX: number,
  clientY: number
): BrowserViewerPoint | null => {
  if (rect.width <= 0 || rect.height <= 0 || frame.width <= 0 || frame.height <= 0) return null;
  const scale = Math.min(rect.width / frame.width, rect.height / frame.height);
  const renderedWidth = frame.width * scale;
  const renderedHeight = frame.height * scale;
  const offsetX = rect.left + (rect.width - renderedWidth) / 2;
  const offsetY = rect.top + (rect.height - renderedHeight) / 2;
  if (
    clientX < offsetX ||
    clientX > offsetX + renderedWidth ||
    clientY < offsetY ||
    clientY > offsetY + renderedHeight
  ) {
    return null;
  }
  return {
    x: Math.max(0, Math.min(frame.width, (clientX - offsetX) / scale)),
    y: Math.max(0, Math.min(frame.height, (clientY - offsetY) / scale)),
  };
};

/** A one-slot buffer: pushing a new value always evicts the old value. */
export class LatestBrowserFrame<T> {
  private value: T | null = null;

  push(next: T): T | null {
    const previous = this.value;
    this.value = next;
    return previous;
  }

  take(): T | null {
    const current = this.value;
    this.value = null;
    return current;
  }

  peek(): T | null {
    return this.value;
  }
}
