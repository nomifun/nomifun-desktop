/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { getI18n } from 'react-i18next';

type Listener = (assetId: string) => void;
const listeners = new WeakMap<object, Set<Listener>>();

export class CreativeAssetDeletedError extends Error {
  constructor(readonly assetId: string) {
    super(getI18n()?.t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' }) ?? '素材已删除');
    this.name = 'CreativeAssetDeletedError';
  }
}

export function notifyCreativeAssetDeleted(client: object, assetId: string): void {
  listeners.get(client)?.forEach((listener) => listener(assetId));
}

export function subscribeCreativeAssetDeletion(client: object, listener: Listener): () => void {
  const subscribers = listeners.get(client) ?? new Set<Listener>();
  listeners.set(client, subscribers);
  subscribers.add(listener);
  return () => { subscribers.delete(listener); };
}
