/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * SessionKnowledgePanel — 会话右栏的知识库预览面板
 *
 * A read-only browser for the knowledge bases a session has mounted. One tree
 * root per mounted base; folders load lazily; clicking a document opens it in
 * the surface's existing preview column.
 *
 * Deliberately read-only: this is a preview entry point, so it imports none of
 * the knowledge mutation calls (`writeFile` / `deleteFile` / `createFolder` /
 * `renameTreeEntry`). Editing stays in `/knowledge`.
 *
 * Rendered through the rail's existing `extraTabs` slot, so it needs no changes
 * to WorkspaceToolRail, WorkspaceRailBody, WorkspacePanelHeader or ChatLayout.
 * It follows the ConversationTerminalPanel precedent: own props, own data, own
 * compact header row, own loading/empty/error states.
 */

import { ipcBridge } from '@/common';
import type { IKnowledgeBase, IKnowledgeTreeEntry } from '@/common/adapter/ipcBridge';
import type { KnowledgeBaseId } from '@/common/types/ids';
import { usePreviewContext } from '@/renderer/pages/conversation/Preview';
import {
  collectKnowledgeDirKeys,
  mergeKnowledgeTreeChildren,
} from '@/renderer/pages/knowledge/KnowledgeDetailPage/treeModel';
import { Button, Empty, Message, Tooltip, Tree } from '@arco-design/web-react';
import { ExpandDown, ExpandUp, Right } from '@icon-park/react';
import classNames from 'classnames';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

/** The rail tab key this panel is registered under. */
export const SESSION_KNOWLEDGE_TAB_KEY = 'session-knowledge';

/**
 * Tree keys must be scoped per knowledge base. Every existing knowledge tree is
 * single-root and keys nodes by bare `rel_path`; with one root per mounted base
 * two bases holding `README.md` would share expand/select state. Base ids are
 * UUIDs and contain no `::`, so the separator is unambiguous.
 */
const KEY_SEP = '::';
const rootKeyOf = (id: KnowledgeBaseId): string => `${id}${KEY_SEP}`;
const nodeKeyOf = (id: KnowledgeBaseId, relPath: string): string => `${id}${KEY_SEP}${relPath}`;

type PanelNode = {
  key: string;
  name: string;
  isLeaf: boolean;
  knowledgeBaseId: KnowledgeBaseId;
  /** '' for a base root. */
  relPath: string;
  children?: PanelNode[];
};

const toPanelNodes = (id: KnowledgeBaseId, entries: IKnowledgeTreeEntry[]): PanelNode[] =>
  entries.map((entry) => ({
    key: nodeKeyOf(id, entry.rel_path),
    name: entry.name,
    isLeaf: entry.is_file,
    knowledgeBaseId: id,
    relPath: entry.rel_path,
    // Only set `children` once a level is actually loaded — an empty array would
    // tell Arco the node is loaded-and-empty and suppress `loadMore`.
    ...(entry.children ? { children: toPanelNodes(id, entry.children) } : {}),
  }));

const SessionKnowledgePanel: React.FC<{ bases: IKnowledgeBase[] }> = ({ bases }) => {
  const { t } = useTranslation();
  const { openPreview } = usePreviewContext();

  const [childrenByBase, setChildrenByBase] = useState<Record<string, IKnowledgeTreeEntry[]>>({});
  const [expandedKeys, setExpandedKeys] = useState<string[]>([]);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [expanding, setExpanding] = useState(false);

  const basesById = useMemo(
    () => new Map(bases.map((base) => [base.knowledge_base_id, base])),
    [bases]
  );

  /** Bases whose source directory is gone cannot be listed at all. */
  const readableBases = useMemo(() => bases.filter((base) => base.root_exists), [bases]);

  const loadLevel = useCallback(
    async (id: KnowledgeBaseId, relPath: string) => {
      const children = await ipcBridge.knowledge.listTree.invoke({
        knowledge_base_id: id,
        ...(relPath ? { path: relPath } : {}),
      });
      setChildrenByBase((previous) => {
        if (!relPath) return { ...previous, [id]: children };
        const current = previous[id] ?? [];
        return { ...previous, [id]: mergeKnowledgeTreeChildren(current, relPath, children) };
      });
    },
    []
  );

  /**
   * Expand every mounted base one level. Deliberately NOT the detail page's
   * recursive `handleExpandAllTreeNodes`: that issues one request per directory
   * with no cache or cancellation, which in a rail with N roots multiplies into
   * dozens. One level per root is N requests and matches the design's
   * "展开：全部根目录".
   */
  const expandAllRoots = useCallback(async () => {
    if (readableBases.length === 0) return;
    setExpanding(true);
    try {
      const levels = await Promise.all(
        readableBases.map(
          async (base) =>
            [
              base.knowledge_base_id,
              await ipcBridge.knowledge.listTree.invoke({ knowledge_base_id: base.knowledge_base_id }),
            ] as const
        )
      );
      setChildrenByBase((previous) => {
        const next = { ...previous };
        for (const [id, children] of levels) next[id] = children;
        return next;
      });
      setExpandedKeys(readableBases.map((base) => rootKeyOf(base.knowledge_base_id)));
    } catch (error) {
      Message.error(String(error));
    } finally {
      setExpanding(false);
    }
  }, [readableBases]);

  // First open shows one level of every root, so the panel never opens on just a
  // couple of collapsed rows. Not persisted — reopening expands again.
  const autoExpandedRef = useRef(false);
  useEffect(() => {
    if (autoExpandedRef.current || readableBases.length === 0) return;
    autoExpandedRef.current = true;
    void expandAllRoots();
  }, [expandAllRoots, readableBases.length]);

  const rootKeys = useMemo(
    () => readableBases.map((base) => rootKeyOf(base.knowledge_base_id)),
    [readableBases]
  );
  const allRootsExpanded = rootKeys.length > 0 && rootKeys.every((key) => expandedKeys.includes(key));

  const handleToggleAll = useCallback(() => {
    if (allRootsExpanded) {
      setExpandedKeys([]);
      return;
    }
    void expandAllRoots();
  }, [allRootsExpanded, expandAllRoots]);

  const treeData = useMemo<PanelNode[]>(
    () =>
      bases.map((base) => {
        const id = base.knowledge_base_id;
        const loaded = childrenByBase[id];
        return {
          key: rootKeyOf(id),
          name: base.name,
          isLeaf: false,
          knowledgeBaseId: id,
          relPath: '',
          ...(base.root_exists && loaded ? { children: toPanelNodes(id, loaded) } : {}),
        };
      }),
    [bases, childrenByBase]
  );

  const openDocument = useCallback(
    async (node: PanelNode) => {
      const base = basesById.get(node.knowledgeBaseId);
      if (!base) return;
      try {
        const file = await ipcBridge.knowledge.readFile.invoke({
          knowledge_base_id: node.knowledgeBaseId,
          path: node.relPath,
        });
        // The backend's tree only ever emits `.md` files (`is_md` gate in
        // nomifun-knowledge), so the preview type is always markdown — no MIME
        // branching needed here.
        openPreview(file.content, 'markdown', {
          title: `${base.name} / ${node.name}`,
          file_name: node.name,
          file_path: `${base.root_path}/${node.relPath}`,
          workspace: base.root_path,
          language: 'md',
          editable: false,
        });
      } catch (error) {
        Message.error(String(error));
      }
    },
    [basesById, openPreview]
  );

  const mountedSummary = t('knowledge.control.mounted', { count: bases.length });
  const toggleLabel = allRootsExpanded
    ? t('knowledge.detail.docs.collapseAll', { defaultValue: '全部折叠' })
    : t('knowledge.detail.docs.expandAll', { defaultValue: '全部展开' });

  return (
    <div className='flex size-full flex-col gap-8px overflow-y-auto p-10px box-border'>
      <div className='flex items-center justify-between gap-8px text-12px text-t-secondary'>
        <span className='min-w-0 truncate'>{mountedSummary}</span>
        <Tooltip content={toggleLabel} position='left' mini>
          <Button
            type='text'
            size='mini'
            shape='circle'
            aria-label={toggleLabel}
            loading={expanding}
            disabled={rootKeys.length === 0}
            icon={
              allRootsExpanded ? (
                <ExpandUp theme='outline' size='14' />
              ) : (
                <ExpandDown theme='outline' size='14' />
              )
            }
            onClick={handleToggleAll}
          />
        </Tooltip>
      </div>

      <Tree
        className='session-knowledge-tree text-13px'
        size='mini'
        blockNode
        showLine
        icons={(nodeProps) => ({
          switcherIcon: nodeProps.isLeaf ? null : (
            <Right
              theme='outline'
              size='11'
              className={classNames('transition-transform duration-150', nodeProps.expanded && 'rotate-90')}
            />
          ),
        })}
        actionOnClick={['select', 'expand']}
        selectedKeys={selectedKey ? [selectedKey] : []}
        expandedKeys={expandedKeys}
        treeData={treeData}
        fieldNames={{ children: 'children', title: 'name', key: 'key', isLeaf: 'isLeaf' }}
        onExpand={(keys) => setExpandedKeys(keys.map(String))}
        onSelect={(_keys, extra) => {
          const dataRef = (extra?.node as { props?: { dataRef?: PanelNode } } | undefined)?.props?.dataRef;
          if (!dataRef) return;
          setSelectedKey(dataRef.key);
          if (dataRef.isLeaf) void openDocument(dataRef);
        }}
        loadMore={(treeNode) => {
          const dataRef = (treeNode.props as { dataRef?: PanelNode }).dataRef;
          if (!dataRef || dataRef.isLeaf) return Promise.resolve();
          const base = basesById.get(dataRef.knowledgeBaseId);
          if (!base?.root_exists) return Promise.resolve();
          return loadLevel(dataRef.knowledgeBaseId, dataRef.relPath).catch((error: unknown) => {
            Message.error(String(error));
          });
        }}
        renderTitle={(node) => {
          const item = node.dataRef as PanelNode;
          const base = item.relPath === '' ? basesById.get(item.knowledgeBaseId) : undefined;
          const rootMissing = Boolean(base && !base.root_exists);
          const emptyRoot =
            Boolean(base?.root_exists) && (childrenByBase[item.knowledgeBaseId]?.length ?? -1) === 0;
          return (
            <span className='flex min-w-0 items-center gap-5px'>
              <span className='block min-w-0 truncate leading-17px' title={item.relPath || item.name}>
                {node.title}
              </span>
              {rootMissing && (
                <span className='shrink-0 text-11px text-danger-6'>
                  {t('knowledge.mount.rootMissing', { defaultValue: '目录不可用' })}
                </span>
              )}
              {emptyRoot && (
                <span className='shrink-0 text-11px text-t-tertiary'>
                  {t('knowledge.session.noDocs', { defaultValue: '没有可预览的文档' })}
                </span>
              )}
            </span>
          );
        }}
      />

      {bases.length === 0 && (
        <div className='flex min-h-160px items-center justify-center'>
          <Empty description={t('knowledge.session.noDocs', { defaultValue: '没有可预览的文档' })} />
        </div>
      )}
    </div>
  );
};

export default SessionKnowledgePanel;
