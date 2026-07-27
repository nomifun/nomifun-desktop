/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest, isBackendHttpError } from '@/common/adapter/httpBridge';
import type { BrowserDisplayMode } from '@/common/config/configKeys';

export type { BrowserDisplayMode } from '@/common/config/configKeys';

export const BROWSER_DISPLAY_MODES = ['embedded', 'external', 'headless'] as const;

export type BrowserDisplayModeMigration = {
  displayMode: BrowserDisplayMode;
  shouldPersist: boolean;
  source: 'displayMode' | 'silent' | 'default';
};

export function isBrowserDisplayMode(value: unknown): value is BrowserDisplayMode {
  return typeof value === 'string' && BROWSER_DISPLAY_MODES.includes(value as BrowserDisplayMode);
}

/**
 * Resolve the current display mode without mutating configuration.
 *
 * The caller owns persistence so this helper remains deterministic and easy to
 * exercise without a renderer or backend:
 * - an explicit valid displayMode wins;
 * - an explicit but malformed displayMode fails safe to embedded and is not
 *   replaced by the legacy key;
 * - legacy silent=false maps to external;
 * - legacy silent=true maps to headless;
 * - a fresh install defaults to embedded.
 */
export function migrateBrowserDisplayMode(input: {
  displayMode?: unknown;
  silent?: unknown;
}): BrowserDisplayModeMigration {
  if (isBrowserDisplayMode(input.displayMode)) {
    return {
      displayMode: input.displayMode,
      shouldPersist: false,
      source: 'displayMode',
    };
  }

  // Presence of the new key is authoritative even when its value is
  // malformed.  The backend follows the same fail-closed rule: a corrupted
  // new value must not be silently reinterpreted through the legacy `silent`
  // preference, otherwise the renderer and backend can boot in different
  // modes.  Keep the malformed value untouched so a later settings write or
  // explicit repair remains the only mutation.
  if (input.displayMode !== undefined) {
    return {
      displayMode: 'embedded',
      shouldPersist: false,
      source: 'displayMode',
    };
  }

  if (input.silent === false) {
    return {
      displayMode: 'external',
      shouldPersist: true,
      source: 'silent',
    };
  }

  if (input.silent === true) {
    return {
      displayMode: 'headless',
      shouldPersist: true,
      source: 'silent',
    };
  }

  return {
    displayMode: 'embedded',
    shouldPersist: true,
    source: 'default',
  };
}

export type BrowserResourcePolicyPreset = 'automatic' | 'resource_saving' | 'high_concurrency';

export type BrowserResourcePolicyAdvanced = {
  max_memory_ratio?: number;
  reserved_memory_bytes?: number;
  max_active_operations?: number;
  max_open_lanes?: number;
  max_queued_requests?: number;
  max_owner_queued_requests?: number;
};

export type BrowserResourcePolicy = {
  preset: BrowserResourcePolicyPreset;
  advanced?: BrowserResourcePolicyAdvanced;
};

export const BROWSER_RESOURCE_POLICY_ADVANCED_FIELDS = [
  'max_memory_ratio',
  'reserved_memory_bytes',
  'max_active_operations',
  'max_open_lanes',
  'max_queued_requests',
  'max_owner_queued_requests',
] as const satisfies readonly (keyof BrowserResourcePolicyAdvanced)[];

const BROWSER_RESOURCE_POLICY_PRESETS = [
  'automatic',
  'resource_saving',
  'high_concurrency',
] as const satisfies readonly BrowserResourcePolicyPreset[];

const RESOURCE_POLICY_PATH = '/api/browser/resource-policy';
const RESOURCE_POLICY_UNAVAILABLE_STATUSES = [404, 501];

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function parseResourcePolicyPreset(value: unknown): BrowserResourcePolicyPreset | undefined {
  return typeof value === 'string' &&
    BROWSER_RESOURCE_POLICY_PRESETS.includes(value as BrowserResourcePolicyPreset)
    ? (value as BrowserResourcePolicyPreset)
    : undefined;
}

function parseFiniteNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

/**
 * Normalize both the final wire shape and the two compatibility shapes used by
 * early backend implementations:
 * - `mode` as an alias for `preset`;
 * - advanced numeric values placed at the top level.
 */
export function normalizeBrowserResourcePolicy(
  raw: unknown,
  fallback: BrowserResourcePolicy = { preset: 'automatic' }
): BrowserResourcePolicy {
  const record = isRecord(raw) ? raw : {};
  const nestedAdvanced = isRecord(record.advanced) ? record.advanced : {};
  const advanced: BrowserResourcePolicyAdvanced = {};

  for (const field of BROWSER_RESOURCE_POLICY_ADVANCED_FIELDS) {
    const value =
      parseFiniteNumber(nestedAdvanced[field]) ??
      parseFiniteNumber(record[field]) ??
      parseFiniteNumber(fallback.advanced?.[field]);
    if (value !== undefined) {
      advanced[field] = value;
    }
  }

  return {
    preset:
      parseResourcePolicyPreset(record.preset) ??
      parseResourcePolicyPreset(record.mode) ??
      fallback.preset,
    advanced: Object.keys(advanced).length > 0 ? advanced : undefined,
  };
}

export function isBrowserResourcePolicyUnavailableError(error: unknown): boolean {
  return isBackendHttpError(error) && RESOURCE_POLICY_UNAVAILABLE_STATUSES.includes(error.status);
}

export const browserResourcePolicyApi = {
  async get(): Promise<BrowserResourcePolicy> {
    const raw = await httpRequest<unknown>('GET', RESOURCE_POLICY_PATH, undefined, {
      silentStatuses: RESOURCE_POLICY_UNAVAILABLE_STATUSES,
    });
    return normalizeBrowserResourcePolicy(raw);
  },

  async put(policy: BrowserResourcePolicy): Promise<BrowserResourcePolicy> {
    const request: BrowserResourcePolicy = {
      preset: policy.preset,
      advanced: policy.advanced,
    };
    const raw = await httpRequest<unknown>('PUT', RESOURCE_POLICY_PATH, request, {
      silentStatuses: RESOURCE_POLICY_UNAVAILABLE_STATUSES,
    });
    return normalizeBrowserResourcePolicy(raw, request);
  },
};
