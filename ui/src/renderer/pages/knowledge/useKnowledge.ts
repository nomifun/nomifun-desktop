/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { createElement, useCallback, useEffect, useRef, useState } from 'react';
import { Message, Notification } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type {
  IKnowledgeAddContentInput,
  IKnowledgeAddContentResult,
  IKnowledgeBase,
  IKnowledgeConsumer,
  IKnowledgeFileEntry,
  IKnowledgeSource,
  IKnowledgeSourceFetchSummary,
  IKnowledgeTreeEntry,
} from '@/common/adapter/ipcBridge';
import type { I18nKey } from '@/renderer/services/i18n';
import type { KnowledgeBaseId } from '@/common/types/ids';

const KNOWLEDGE_OPEN_PREFETCH_TTL_MS = 30_000;

interface KnowledgeOpenPrefetch {
  base: IKnowledgeBase;
  treePromise: Promise<IKnowledgeTreeEntry[]>;
  expiresAt: number;
}

const knowledgeOpenPrefetches = new Map<string, KnowledgeOpenPrefetch>();

const readKnowledgeOpenPrefetch = (
  id: KnowledgeBaseId | undefined
): KnowledgeOpenPrefetch | undefined => {
  if (!id) return undefined;
  const cached = knowledgeOpenPrefetches.get(id);
  if (!cached) return undefined;
  if (cached.expiresAt > Date.now()) return cached;
  knowledgeOpenPrefetches.delete(id);
  return undefined;
};

/**
 * Start the cheap root-tree request at navigation intent time. The list's
 * already-materialised base summary is retained as the opening snapshot, so a
 * card click does not immediately repeat the backend's full directory stats
 * walk before it can render the detail shell.
 */
export function prefetchKnowledgeBaseOpen(
  base: IKnowledgeBase
): Promise<IKnowledgeTreeEntry[]> {
  const id = base.knowledge_base_id;
  const cached = readKnowledgeOpenPrefetch(id);
  if (cached && cached.base.updated_at === base.updated_at) {
    cached.base = base;
    cached.expiresAt = Date.now() + KNOWLEDGE_OPEN_PREFETCH_TTL_MS;
    return cached.treePromise;
  }

  const treePromise = ipcBridge.knowledge.listTree.invoke({
    knowledge_base_id: id,
  });
  const entry: KnowledgeOpenPrefetch = {
    base,
    treePromise,
    expiresAt: Date.now() + KNOWLEDGE_OPEN_PREFETCH_TTL_MS,
  };
  knowledgeOpenPrefetches.set(id, entry);
  void treePromise.catch(() => {
    if (knowledgeOpenPrefetches.get(id) === entry) {
      knowledgeOpenPrefetches.delete(id);
    }
  });
  return treePromise;
}

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

export function useKnowledgeBase(
  id: KnowledgeBaseId | undefined,
  openingBase?: IKnowledgeBase
) {
  const openingSnapshot =
    openingBase?.knowledge_base_id === id
      ? openingBase
      : readKnowledgeOpenPrefetch(id)?.base;
  const [base, setBase] = useState<IKnowledgeBase | null>(
    openingSnapshot ?? null
  );
  const [files, setFiles] = useState<IKnowledgeFileEntry[]>([]);
  const [tree, setTree] = useState<IKnowledgeTreeEntry[]>([]);
  const [loading, setLoading] = useState(Boolean(id));
  const [filesLoading, setFilesLoading] = useState(false);
  const [filesLoaded, setFilesLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestRef = useRef(0);
  const activeIdRef = useRef(id);
  activeIdRef.current = id;
  const filesRequestRef = useRef<{
    id: KnowledgeBaseId;
    promise: Promise<IKnowledgeFileEntry[]>;
  } | null>(null);
  const filesRequestGenerationRef = useRef(0);
  const filesLoadedRef = useRef(false);

  const loadFiles = useCallback(async (): Promise<IKnowledgeFileEntry[]> => {
    if (!id) return [];
    if (filesRequestRef.current?.id === id) {
      return filesRequestRef.current.promise;
    }

    setFilesLoading(true);
    const generation = ++filesRequestGenerationRef.current;
    const promise = (async () => {
      try {
        const list = await ipcBridge.knowledge.listFiles.invoke({
          knowledge_base_id: id,
        });
        if (
          activeIdRef.current === id &&
          generation === filesRequestGenerationRef.current
        ) {
          setFiles(list);
          filesLoadedRef.current = true;
          setFilesLoaded(true);
        }
        return list;
      } catch (e) {
        if (
          activeIdRef.current === id &&
          generation === filesRequestGenerationRef.current
        ) {
          console.error('Failed to load knowledge files', e);
          setError(String(e));
        }
        throw e;
      } finally {
        if (
          filesRequestRef.current?.id === id &&
          generation === filesRequestGenerationRef.current
        ) {
          filesRequestRef.current = null;
        }
        if (
          activeIdRef.current === id &&
          generation === filesRequestGenerationRef.current
        ) {
          setFilesLoading(false);
        }
      }
    })();
    filesRequestRef.current = { id, promise };
    return promise;
  }, [id]);

  const refresh = useCallback(async () => {
    if (!id) return;
    const request = ++requestRef.current;
    setLoading(true);
    try {
      // Root navigation is the first useful paint. It must win the backend's
      // per-root filesystem lock before the slower recursive stats/files walk.
      const treeRoot = await ipcBridge.knowledge.listTree.invoke({
        knowledge_base_id: id,
      });
      if (request !== requestRef.current) return;
      setTree(treeRoot);
      setLoading(false);

      const info = await ipcBridge.knowledge.getBase.invoke({
        knowledge_base_id: id,
      });
      if (request !== requestRef.current) return;
      setBase(info);
      setError(null);

      if (filesLoadedRef.current) {
        filesRequestRef.current = null;
        filesRequestGenerationRef.current += 1;
        await loadFiles();
      }
    } catch (e) {
      if (request !== requestRef.current) return;
      console.error('Failed to load knowledge base', e);
      setError(String(e));
    } finally {
      if (request === requestRef.current) setLoading(false);
    }
  }, [id, loadFiles]);

  useEffect(() => {
    const request = ++requestRef.current;
    filesLoadedRef.current = false;
    filesRequestRef.current = null;
    filesRequestGenerationRef.current += 1;
    setFiles([]);
    setFilesLoaded(false);
    setFilesLoading(false);
    setError(null);

    if (!id) {
      setBase(null);
      setTree([]);
      setLoading(false);
      return;
    }

    const prefetched = readKnowledgeOpenPrefetch(id);
    const snapshot =
      openingBase?.knowledge_base_id === id ? openingBase : prefetched?.base;
    setBase(snapshot ?? null);
    setTree([]);
    setLoading(true);

    void (async () => {
      try {
        const treeRoot = await (
          prefetched?.treePromise ??
          ipcBridge.knowledge.listTree.invoke({ knowledge_base_id: id })
        );
        if (request !== requestRef.current) return;
        setTree(treeRoot);
        // Card navigations already carry authoritative list metadata, so the
        // detail view is usable as soon as its root tree arrives.
        if (snapshot) {
          setLoading(false);
          return;
        }

        // Direct deep links have no list snapshot. Fetch metadata only after
        // the root tree has won the per-root lock and become renderable.
        const info = await ipcBridge.knowledge.getBase.invoke({
          knowledge_base_id: id,
        });
        if (request !== requestRef.current) return;
        setBase(info);
        setLoading(false);
      } catch (e) {
        if (request !== requestRef.current) return;
        console.error('Failed to load knowledge base', e);
        setError(String(e));
        setLoading(false);
      }
    })();

    return () => {
      if (request === requestRef.current) requestRef.current += 1;
    };
  }, [id, openingBase]);

  // Keep the detail view in sync with backend-side updates (autogen /
  // snapshot refresh / gateway edits all broadcast knowledge.base-updated).
  useEffect(() => {
    if (!id) return;
    const unsub = ipcBridge.knowledge.onBaseUpdated.on((b) => {
      if (b.knowledge_base_id === id) void refresh();
    });
    return () => unsub();
  }, [id, refresh]);

  return {
    base,
    files,
    tree,
    loading,
    filesLoading,
    filesLoaded,
    error,
    refresh,
    loadFiles,
  };
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

/**
 * Type-safe frontend boundary for the unified append-only content endpoint.
 * The lower bridge uses optional fields because mapped invoke types cannot
 * retain a discriminated union; callers stay strict by going through here.
 */
export function addKnowledgeContent(
  knowledgeBaseId: KnowledgeBaseId,
  input: IKnowledgeAddContentInput,
): Promise<IKnowledgeAddContentResult> {
  return ipcBridge.knowledge.addContent.invoke({
    knowledge_base_id: knowledgeBaseId,
    ...input,
  });
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
