/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { createElement, useCallback, useEffect, useState } from 'react';
import { Message, Notification } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type {
  IKnowledgeBase,
  IKnowledgeConsumer,
  IKnowledgeFileEntry,
  IKnowledgeSource,
  IKnowledgeSourceFetchSummary,
  IKnowledgeTreeEntry,
} from '@/common/adapter/ipcBridge';
import type { I18nKey } from '@/renderer/services/i18n';
import type { KnowledgeBaseId } from '@/common/types/ids';

export function useKnowledgeBases() {
  const [bases, setBases] = useState<IKnowledgeBase[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const res = await ipcBridge.knowledge.listBases.invoke();
      setBases(res);
      setError(null);
    } catch (e) {
      console.error('Failed to load knowledge bases', e);
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const unsubs = [
      ipcBridge.knowledge.onBaseCreated.on(() => void refresh()),
      ipcBridge.knowledge.onBaseUpdated.on(() => void refresh()),
      ipcBridge.knowledge.onBaseDeleted.on(() => void refresh()),
    ];
    return () => unsubs.forEach((u) => u());
  }, [refresh]);

  return { bases, loading, error, refresh };
}

export function useKnowledgeBase(id: KnowledgeBaseId | undefined) {
  const [base, setBase] = useState<IKnowledgeBase | null>(null);
  const [files, setFiles] = useState<IKnowledgeFileEntry[]>([]);
  const [tree, setTree] = useState<IKnowledgeTreeEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!id) return;
    setLoading(true);
    try {
      const [info, list, treeRoot] = await Promise.all([
        ipcBridge.knowledge.getBase.invoke({ knowledge_base_id: id }),
        ipcBridge.knowledge.listFiles.invoke({ knowledge_base_id: id }),
        ipcBridge.knowledge.listTree.invoke({ knowledge_base_id: id }),
      ]);
      setBase(info);
      setFiles(list);
      setTree(treeRoot);
      setError(null);
    } catch (e) {
      console.error('Failed to load knowledge base', e);
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Keep the detail view in sync with backend-side updates (autogen /
  // snapshot refresh / gateway edits all broadcast knowledge.base-updated).
  useEffect(() => {
    if (!id) return;
    const unsub = ipcBridge.knowledge.onBaseUpdated.on((b) => {
      if (b.knowledge_base_id === id) void refresh();
    });
    return () => unsub();
  }, [id, refresh]);

  return { base, files, tree, loading, error, refresh };
}

/** Bindings (workspaces/conversations/…) currently mounting a base. */
export function useKnowledgeConsumers(id: KnowledgeBaseId | undefined) {
  const [consumers, setConsumers] = useState<IKnowledgeConsumer[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!id) return;
    setLoading(true);
    try {
      const res = await ipcBridge.knowledge.listConsumers.invoke({ knowledge_base_id: id });
      setConsumers(res);
      setError(null);
    } catch (e) {
      console.error('Failed to load knowledge consumers', e);
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const unsub = ipcBridge.knowledge.onBindingChanged.on(() => void refresh());
    return () => unsub();
  }, [refresh]);

  return { consumers, loading, error, refresh };
}

/** Null-safe accessor for a base's URL source config (top-level `source` on the wire). */
export function getBaseSource(base: IKnowledgeBase | null | undefined): IKnowledgeSource | undefined {
  return base?.source;
}

/** Human-readable message for a knowledge API failure (prefers the backend-provided message). */
export function knowledgeErrorText(e: unknown): string {
  if (isBackendHttpError(e) && e.backendMessage.trim()) return e.backendMessage;
  return e instanceof Error ? e.message : String(e);
}

/** True when the error is the autogen 409 — no AI completer/provider configured. */
export function isAutogenNoProviderError(e: unknown): boolean {
  return isBackendHttpError(e) && e.status === 409;
}

type TranslateFn = (key: I18nKey, options?: Record<string, unknown>) => string;

/**
 * Surface a URL-source fetch outcome (create-time `source_fetch` / refresh-source
 * response). Failures get a sticky notification listing each failed URL; a fully
 * successful run shows `okMessage` when provided (callers pass none at create
 * time, where the regular "created" toast already covers it).
 */
export function notifySourceFetchResult(t: TranslateFn, summary: IKnowledgeSourceFetchSummary, okMessage?: string): void {
  if (summary.failed > 0) {
    Notification.warning({
      title: t('knowledge.source.fetchFailedTitle'),
      content: createElement(
        'div',
        { className: 'flex flex-col gap-4px max-h-220px overflow-y-auto' },
        createElement(
          'span',
          { key: 'summary' },
          t('knowledge.source.fetchSummary', { fetched: summary.fetched, failed: summary.failed })
        ),
        ...summary.errors.map((line, i) => createElement('span', { key: i, className: 'text-12px break-all' }, line))
      ),
      duration: 10000,
    });
  } else if (okMessage) {
    Message.success(okMessage);
  }
}

/** Render a byte count as a short human-readable size. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
