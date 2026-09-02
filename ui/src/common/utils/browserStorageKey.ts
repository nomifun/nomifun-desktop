/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  EntityId,
  EntityKind,
  SessionTarget,
} from '@/common/types/ids';
import { parseEntityId } from '@/common/types/ids';
import { uuidv7 } from './uuidv7';

export const BROWSER_STORAGE_SCHEMA_VERSION = 1 as const;
export const BROWSER_STORAGE_GENERATION_STORAGE_KEY = 'nomifun_browser_storage_generation_v1';

export type BrowserStorageEntityKind = EntityKind;

export type BrowserStorageFeature =
  | 'workspace-collapse'
  | 'workspace-panel-tab'
  | 'workspace-preview'
  | 'draft'
  | 'initial-message-nomi'
  | 'initial-message-processed'
  | 'command-queue'
  | 'cron-unread'
  | (string & {});

export type BrowserStoragePersistence = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

const KEY_ROOT = 'nomifun';
let storageGeneration: string | null = null;
let provisionalStorageGeneration: string | null = null;

/**
 * Sets the identity of the currently mounted backend dataset.
 *
 * Call this with `application.systemInfo.storageGeneration` during renderer
 * bootstrap. Keeping the generation in every entity-scoped key prevents
 * browser state surviving a reset or restore from binding to a new graph.
 */
export function setBrowserStorageGeneration(value: string): void {
  try {
    parseEntityId('user', value);
  } catch {
    throw new TypeError('storage generation must be a canonical lowercase UUIDv7 string');
  }
  storageGeneration = value;
}

function isCanonicalStorageGeneration(value: unknown): value is string {
  if (typeof value !== 'string') return false;
  try {
    parseEntityId('user', value);
    return true;
  } catch {
    return false;
  }
}

function defaultBrowserStorage(): BrowserStoragePersistence | undefined {
  try {
    return typeof localStorage === 'undefined' ? undefined : localStorage;
  } catch {
    return undefined;
  }
}

function readPersistedStorageGeneration(storage: BrowserStoragePersistence | undefined): string | null {
  if (!storage) return null;

  try {
    const value = storage.getItem(BROWSER_STORAGE_GENERATION_STORAGE_KEY);
    if (value === null) return null;
    if (isCanonicalStorageGeneration(value)) return value;

    // Do not let a legacy or corrupted value poison the next bootstrap.
    try {
      storage.removeItem(BROWSER_STORAGE_GENERATION_STORAGE_KEY);
    } catch {
      // Storage cleanup is best effort; the generated value still remains safe.
    }
  } catch {
    // Storage can be unavailable in a sandboxed or opaque-origin webview.
  }

  return null;
}

function persistStorageGeneration(storage: BrowserStoragePersistence | undefined, value: string): void {
  if (!storage) return;
  try {
    storage.setItem(BROWSER_STORAGE_GENERATION_STORAGE_KEY, value);
  } catch {
    // Browser persistence is an optimization; in-memory initialization remains valid.
  }
}

/**
 * Initialize the generation used by browser-local state during renderer
 * bootstrap.
 *
 * The backend value is authoritative whenever it is valid. A malformed or
 * temporarily unavailable backend value must not crash the renderer before
 * the backend can finish its bootstrap, so a previously persisted canonical
 * value is used as a provisional fallback. If no usable value exists, mint a
 * fresh UUIDv7 with WebCrypto and persist it when browser storage is usable.
 *
 * `setBrowserStorageGeneration` remains strict; this function never normalizes
 * or accepts a non-canonical value.
 */
export function initializeBrowserStorageGeneration(
  backendValue: unknown,
  storage: BrowserStoragePersistence | undefined = defaultBrowserStorage(),
): string {
  if (isCanonicalStorageGeneration(backendValue)) {
    setBrowserStorageGeneration(backendValue);
    provisionalStorageGeneration = null;
    persistStorageGeneration(storage, backendValue);
    return backendValue;
  }

  // Keep one provisional generation for this renderer lifetime when browser
  // persistence is unavailable or the backend is still bootstrapping.
  if (!storage && provisionalStorageGeneration) {
    setBrowserStorageGeneration(provisionalStorageGeneration);
    return provisionalStorageGeneration;
  }

  const persisted = readPersistedStorageGeneration(storage);
  if (persisted) {
    setBrowserStorageGeneration(persisted);
    provisionalStorageGeneration = persisted;
    return persisted;
  }

  const generated = uuidv7();
  setBrowserStorageGeneration(generated);
  provisionalStorageGeneration = generated;
  persistStorageGeneration(storage, generated);
  return generated;
}

export function getBrowserStorageGeneration(): string {
  if (!storageGeneration) {
    throw new Error('browser storage generation has not been initialized');
  }
  return storageGeneration;
}

/** A generation-scoped key for UI state that is not owned by one entity. */
export function browserStorageGenerationKey(feature: BrowserStorageFeature): string {
  return [
    KEY_ROOT,
    `v${BROWSER_STORAGE_SCHEMA_VERSION}`,
    encodeSegment(getBrowserStorageGeneration()),
    encodeSegment(feature),
  ].join('|');
}

function encodeSegment(value: string): string {
  return `${value.length}:${value}`;
}

/**
 * Produces an unambiguous, versioned entity-scoped browser storage key.
 *
 * Length-prefixed segments ensure tuples such as (`ab`, `c`) and (`a`, `bc`)
 * can never collide. Entity kind is mandatory, so conversation "1" and
 * terminal "1" occupy distinct namespaces.
 */
export function browserStorageKey<Kind extends EntityKind>(
  feature: BrowserStorageFeature,
  entityKind: Kind,
  entityId: EntityId<Kind>
): string;
export function browserStorageKey(
  feature: BrowserStorageFeature,
  entityKind: BrowserStorageEntityKind,
  entityId: string
): string {
  const generation = getBrowserStorageGeneration();
  return [
    KEY_ROOT,
    `v${BROWSER_STORAGE_SCHEMA_VERSION}`,
    encodeSegment(generation),
    encodeSegment(feature),
    encodeSegment(entityKind),
    encodeSegment(String(entityId)),
  ].join('|');
}

export function sessionStorageKey(feature: BrowserStorageFeature, target: SessionTarget): string {
  return browserStorageKey(feature, target.kind, target.id);
}
