/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { EntityId, EntityKind, SessionTarget } from '@/common/types/ids';

export const BROWSER_STORAGE_SCHEMA_VERSION = 1 as const;

export type BrowserStorageEntityKind = EntityKind;

export type BrowserStorageFeature =
  | 'workspace-collapse'
  | 'workspace-panel-tab'
  | 'workspace-preview'
  | 'draft'
  | 'initial-message-acp'
  | 'initial-message-nanobot'
  | 'initial-message-nomi'
  | 'initial-message-openclaw'
  | 'initial-message-remote'
  | 'initial-message-processed'
  | 'command-queue'
  | (string & {});

const KEY_ROOT = 'nomifun';
let storageGeneration: string | null = null;

/**
 * Sets the identity of the currently mounted backend dataset.
 *
 * Call this with `application.systemInfo.storageGeneration` during renderer
 * bootstrap. Keeping the generation in every entity-scoped key prevents
 * browser state surviving a reset or restore from binding to a new graph.
 */
export function setBrowserStorageGeneration(value: string): void {
  if (
    value.trim() !== value ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value)
  ) {
    throw new TypeError('storage generation must be a canonical lowercase UUIDv7 string');
  }
  storageGeneration = value;
}

export function getBrowserStorageGeneration(): string {
  if (!storageGeneration) {
    throw new Error('browser storage generation has not been initialized');
  }
  return storageGeneration;
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
export function browserStorageKey<Kind extends BrowserStorageEntityKind>(
  feature: BrowserStorageFeature,
  entityKind: Kind,
  entityId: EntityId<Kind>
): string {
  const generation = getBrowserStorageGeneration();
  return [
    KEY_ROOT,
    `v${BROWSER_STORAGE_SCHEMA_VERSION}`,
    encodeSegment(generation),
    encodeSegment(feature),
    encodeSegment(entityKind),
    encodeSegment(entityId),
  ].join('|');
}

export function sessionStorageKey(feature: BrowserStorageFeature, target: SessionTarget): string {
  return browserStorageKey(feature, target.kind, target.id);
}
