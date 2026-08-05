/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Modal } from '@arco-design/web-react';
import ContentAside from '@/renderer/components/layout/ContentAside';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import KnowledgeControl from '@/renderer/pages/conversation/components/KnowledgeControl';
import type { ICompanionMemory, ICompanionMemoryBatchAction, ICompanionMemoryKind } from '@/common/adapter/ipcBridge';
import { useAsidePortal } from '../../AsideHost';
import type { WorkspaceTabProps } from '../../types';
import MemoryComposePane from './MemoryComposePane';
import MemoryDetailPane from './MemoryDetailPane';
import MemoryList from './MemoryList';
import MemoryToolbar from './MemoryToolbar';
import MergeAssistantPane from './MergeAssistantPane';
import { MEMORY_KINDS, formatMemoryTime } from './constants';
import { useMemoryList } from './useMemoryList';

/** What the right-hand pane is showing. `null` = closed. */
type PaneMode = 'detail' | 'compose' | 'merge' | null;

/**
 * 记忆&知识库 — the memory this companion can consult, plus its knowledge bases.
 *
 * Every request is scoped to `companionId`, reads and writes alike: the list is
 * exactly what this companion sees at retrieval time, and each mutation says who
 * is asking, so the server refuses anything that is not this companion's. There
 * is no scope picker and no owner column because there is nothing to pick —
 * memory belongs to one companion from the moment it is written.
 */
const MemoryTab: React.FC<WorkspaceTabProps> = ({ companionId, companion, onAttentionChange }) => {
  const { t } = useTranslation();
  const list = useMemoryList(companionId);
  const { profile } = companion;

  const [pane, setPane] = useState<PaneMode>(null);
  const [detail, setDetail] = useState<ICompanionMemory | null>(null);
  const [reclassifyOpen, setReclassifyOpen] = useState(false);
  const [reclassifyKind, setReclassifyKind] = useState<ICompanionMemoryKind>('knowledge');

  // Switching companion invalidates whatever the pane was showing.
  useEffect(() => {
    setPane(null);
    setDetail(null);
  }, [companionId]);

  // Nothing in this tab waits on the user: memory is a place you go to look, not
  // an inbox. Report "no attention" once so the strip never carries a stale dot.
  useEffect(() => {
    onAttentionChange?.(false);
  }, [onAttentionChange]);

  // Keep the open memory in step with the list (pin/archive/edit from elsewhere).
  // A memory that drops out of the current result set stays open on its last
  // known value rather than yanking the pane shut mid-edit.
  useEffect(() => {
    setDetail((prev) => (prev ? (list.items.find((m) => m.memory_id === prev.memory_id) ?? prev) : prev));
  }, [list.items]);

  const closePane = useCallback(() => {
    setPane(null);
    setDetail(null);
  }, []);

  const openDetail = useCallback((memory: ICompanionMemory) => {
    setDetail(memory);
    setPane('detail');
  }, []);

  const openCompose = useCallback(() => {
    setDetail(null);
    setPane('compose');
  }, []);

  const toggleMerge = useCallback(() => {
    setDetail(null);
    setPane((prev) => (prev === 'merge' ? null : 'merge'));
  }, []);

  const confirmDelete = useCallback(
    (memory: ICompanionMemory) => {
      Modal.confirm({
        title: t('nomi.memories.delete', { defaultValue: '删除' }),
        // Memory is per-companion, so a delete only ever affects this companion.
        content: t('nomi.memories.deleteConfirm', { defaultValue: '确定永久删除这条记忆？' }),
        okButtonProps: { status: 'danger' },
        onOk: async () => {
          try {
            await list.removeMemory(memory.memory_id);
            if (detail?.memory_id === memory.memory_id) closePane();
          } catch (e) {
            Message.error(String(e));
          }
        },
      });
    },
    [closePane, detail?.memory_id, list, t]
  );

  const runBatch = useCallback(
    async (action: ICompanionMemoryBatchAction, kind?: ICompanionMemoryKind) => {
      try {
        // `0` = the selection was empty and nothing was sent; never claim success.
        const count = await list.runBatch(action, kind);
        if (count > 0) Message.success(t('nomi.memories.batchDone', { defaultValue: '批量操作完成' }));
      } catch (e) {
        Message.error(String(e));
      }
    },
    [list, t]
  );

  const confirmBatch = useCallback(
    (action: 'archive' | 'restore' | 'delete') => {
      const count = list.selected.length;
      const title =
        action === 'archive'
          ? t('nomi.memories.batchArchiveConfirm', { count, defaultValue: '归档选中的 {{count}} 条记忆？' })
          : action === 'restore'
            ? t('nomi.memories.batchRestoreConfirm', { count, defaultValue: '恢复选中的 {{count}} 条记忆？' })
            : t('nomi.memories.batchDeleteConfirm', { count, defaultValue: '永久删除选中的 {{count}} 条记忆？' });
      Modal.confirm({
        title,
        okButtonProps: action === 'delete' ? { status: 'danger' } : undefined,
        onOk: () => void runBatch(action),
      });
    },
    [list.selected.length, runBatch, t]
  );

  const submitReclassify = useCallback(async () => {
    setReclassifyOpen(false);
    await runBatch('reclassify', reclassifyKind);
  }, [reclassifyKind, runBatch]);

  /**
   * Pin/archive from the pane must patch the open memory itself. Archiving while
   * the list is filtered to `active` drops the row out of the result set, so the
   * sync effect above has nothing to re-read and the switch would otherwise snap
   * straight back to its old position on a request that actually succeeded.
   * `memory` is the pre-mutation row, so a failure restores it exactly.
   */
  const patchDetail = useCallback(
    async (memory: ICompanionMemory, patch: Partial<ICompanionMemory>, run: () => Promise<void>) => {
      const sameRow = (prev: ICompanionMemory | null) => prev != null && prev.memory_id === memory.memory_id;
      setDetail((prev) => (sameRow(prev) ? { ...(prev as ICompanionMemory), ...patch } : prev));
      try {
        await run();
      } catch (e) {
        setDetail((prev) => (sameRow(prev) ? memory : prev));
        Message.error(String(e));
      }
    },
    []
  );

  const paneNode = (() => {
    if (pane === 'detail' && detail) {
      return (
        <ContentAside
          title={t('nomi.memory.detailTitle', { defaultValue: '记忆详情' })}
          subtitle={`${t(`nomi.kinds.${detail.kind}`)} · ${formatMemoryTime(detail.updated_at)}`}
          onClose={closePane}
          storageKey='nomifun:nomi-aside-memory'
        >
          <MemoryDetailPane
            memory={detail}
            onSave={(content) => list.saveContent(detail.memory_id, content)}
            onTogglePin={(pinned) => patchDetail(detail, { pinned }, () => list.setPinned(detail, pinned))}
            onToggleArchive={(archived) =>
              patchDetail(detail, { status: archived ? 'archived' : 'active' }, () =>
                list.setArchived(detail, archived)
              )
            }
            onDelete={() => confirmDelete(detail)}
          />
        </ContentAside>
      );
    }
    if (pane === 'compose') {
      return (
        <ContentAside
          title={t('nomi.memories.add', { defaultValue: '添加记忆' })}
          subtitle={t('nomi.memory.composeSubtitle', { defaultValue: '只属于这个伙伴，别的伙伴看不到' })}
          onClose={closePane}
          storageKey='nomifun:nomi-aside-memory'
        >
          <MemoryComposePane onSubmit={list.addMemory} onDone={closePane} />
        </ContentAside>
      );
    }
    if (pane === 'merge') {
      return (
        <ContentAside
          title={t('nomi.memories.mergeTitle', { defaultValue: '查重合并助手' })}
          subtitle={t('nomi.memory.mergeSubtitle', { defaultValue: '把说的是同一件事的记忆并成一条' })}
          onClose={closePane}
          storageKey='nomifun:nomi-aside-memory'
        >
          <MergeAssistantPane companionId={companionId} onMerged={() => void list.refresh()} />
        </ContentAside>
      );
    }
    return null;
  })();

  const aside = useAsidePortal(paneNode);

  return (
    <>
      <div className='flex flex-col gap-16px'>
        <NomiSettingSection
          title={t('nomi.memory.memorySection', { defaultValue: '记忆' })}
          description={t('nomi.memory.memorySectionHint', {
            defaultValue: '这个伙伴在对话时能检索到的记忆。点开一条可以直接改内容、置顶或归档。',
          })}
        >
          <div className='flex flex-col gap-10px'>
            <MemoryToolbar
              q={list.q}
              onQChange={list.setQ}
              kind={list.kind}
              onKindChange={list.setKind}
              status={list.status}
              onStatusChange={list.setStatus}
              sort={list.sort}
              onSortChange={list.setSort}
              onOpenMerge={toggleMerge}
              mergeOpen={pane === 'merge'}
              onAdd={openCompose}
            />
            <MemoryList
              list={list}
              activeId={pane === 'detail' ? (detail?.memory_id ?? null) : null}
              onOpen={openDetail}
              onDelete={confirmDelete}
              onAdd={openCompose}
              onBatch={confirmBatch}
              onReclassify={() => setReclassifyOpen(true)}
            />
          </div>
        </NomiSettingSection>

        <NomiSettingSection title={t('nomi.settings.knowledge', { defaultValue: '知识库' })}>
          <NomiSettingList>
            <NomiSettingRow
              title={t('nomi.settings.knowledge', { defaultValue: '知识库' })}
              description={
                profile
                  ? t('nomi.settings.knowledgeHint', {
                      companionName: profile.name,
                      defaultValue: '为 {{companionName}} 挂载知识库，对话时可以查阅与回写',
                    })
                  : undefined
              }
              controls={<KnowledgeControl target={{ kind: 'companion', id: companionId }} />}
            />
          </NomiSettingList>
        </NomiSettingSection>
      </div>

      <Modal
        title={t('nomi.memories.reclassifyTitle', { defaultValue: '批量改分类' })}
        visible={reclassifyOpen}
        onOk={() => void submitReclassify()}
        onCancel={() => setReclassifyOpen(false)}
      >
        <div className='flex flex-col gap-12px'>
          <div className='text-13px leading-20px text-t-secondary'>
            {t('nomi.memories.reclassifyPick', {
              count: list.selected.length,
              defaultValue: '为选中的 {{count}} 条记忆选择新分类',
            })}
          </div>
          <NomiSelect value={reclassifyKind} onChange={(value: ICompanionMemoryKind) => setReclassifyKind(value)}>
            {MEMORY_KINDS.map((item) => (
              <NomiSelect.Option key={item} value={item}>
                {t(`nomi.kinds.${item}`)}
              </NomiSelect.Option>
            ))}
          </NomiSelect>
        </div>
      </Modal>

      {aside}
    </>
  );
};

export default MemoryTab;
