/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Pagination, Spin } from '@arco-design/web-react';
import BatchActionBar from '@/renderer/components/base/BatchActionBar';
import type { ICompanionMemory } from '@/common/adapter/ipcBridge';
import type { CompanionMemoryId } from '@/common/types/ids';
import MemoryListEmpty from './MemoryListEmpty';
import MemoryListRow from './MemoryListRow';
import { MEMORY_PAGE_SIZE_OPTIONS } from './constants';
import type { MemoryListHandle } from './useMemoryList';

interface MemoryListProps {
  list: MemoryListHandle;
  /** The memory whose detail pane is open, if any. */
  activeId: CompanionMemoryId | null;
  onOpen: (memory: ICompanionMemory) => void;
  onDelete: (memory: ICompanionMemory) => void;
  onAdd: () => void;
  onBatch: (action: 'archive' | 'restore' | 'delete') => void;
  onReclassify: () => void;
}

/**
 * The memory list body: batch bar, hairline rows, pagination. Kept separate from
 * the tab shell so the shell stays a map of sections.
 */
const MemoryList: React.FC<MemoryListProps> = ({ list, activeId, onOpen, onDelete, onAdd, onBatch, onReclassify }) => {
  const { t } = useTranslation();
  const { items, total, loading, selected, page, pageSize, status } = list;

  const hasSelection = selected.length > 0;
  const filtered = Boolean(list.q.trim()) || list.kind !== '' || status !== 'active';

  const actions = useMemo(() => {
    type Action = { key: string; label: string; onClick: () => void; danger?: boolean };
    if (!hasSelection) return [] as Action[];
    const result: Action[] = [];
    if (status !== 'archived') {
      result.push({
        key: 'archive',
        label: t('nomi.memories.batchArchive', { defaultValue: '批量归档' }),
        onClick: () => onBatch('archive'),
      });
    }
    if (status !== 'active') {
      result.push({
        key: 'restore',
        label: t('nomi.memories.batchRestore', { defaultValue: '批量恢复' }),
        onClick: () => onBatch('restore'),
      });
    }
    result.push({
      key: 'reclassify',
      label: t('nomi.memories.batchReclassify', { defaultValue: '改分类' }),
      onClick: onReclassify,
    });
    result.push({
      key: 'delete',
      label: t('nomi.memories.batchDelete', { defaultValue: '批量删除' }),
      onClick: () => onBatch('delete'),
      danger: true,
    });
    return result;
  }, [hasSelection, status, t, onBatch, onReclassify]);

  if (loading && items.length === 0) {
    return (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    );
  }

  if (items.length === 0) {
    return <MemoryListEmpty filtered={filtered} onAdd={onAdd} />;
  }

  return (
    <div className='flex flex-col'>
      <BatchActionBar
        selectAllLabel={
          hasSelection
            ? t('nomi.memory.selectedClear', { count: selected.length, defaultValue: '已选 {{count}} 条 · 清除' })
            : t('nomi.memories.selectAll', { defaultValue: '全选本页' })
        }
        onSelectAll={hasSelection ? list.clearSelection : list.selectAllOnPage}
        actions={actions}
      />

      {/* A stale result set dims rather than blanking, so the page never jumps. */}
      <div className='flex flex-col transition-opacity duration-150' style={{ opacity: loading ? 0.6 : 1 }}>
        {items.map((memory) => (
          <MemoryListRow
            key={memory.memory_id}
            memory={memory}
            checked={selected.includes(memory.memory_id)}
            active={activeId === memory.memory_id}
            onToggleSelect={list.toggleSelected}
            onOpen={onOpen}
            onTogglePin={(m) => void list.setPinned(m, !m.pinned).catch((e) => Message.error(String(e)))}
            onToggleArchive={(m) =>
              void list.setArchived(m, m.status === 'active').catch((e) => Message.error(String(e)))
            }
            onDelete={onDelete}
          />
        ))}
      </div>

      <div className='mt-10px flex flex-wrap items-center justify-between gap-10px'>
        <span className='text-12px leading-18px text-t-tertiary tabular-nums'>
          {t('nomi.memories.total', { count: total, defaultValue: '共 {{count}} 条记忆' })}
        </span>
        <Pagination
          size='small'
          current={page}
          pageSize={pageSize}
          total={total}
          sizeCanChange
          sizeOptions={MEMORY_PAGE_SIZE_OPTIONS}
          showJumper={total > pageSize}
          onChange={list.handlePageChange}
        />
      </div>
    </div>
  );
};

export default MemoryList;
