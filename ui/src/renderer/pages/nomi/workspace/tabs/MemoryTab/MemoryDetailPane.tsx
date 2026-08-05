/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Input, Message, Switch } from '@arco-design/web-react';
import type { ICompanionMemory } from '@/common/adapter/ipcBridge';
import { NomiSettingList, NomiSettingRow } from '@/renderer/components/base/NomiSettingLayout';
import { MEMORY_KIND_DOT, formatMemoryTime } from './constants';

interface MemoryDetailPaneProps {
  memory: ICompanionMemory;
  onSave: (content: string) => Promise<void>;
  onTogglePin: (pinned: boolean) => Promise<void>;
  onToggleArchive: (archived: boolean) => Promise<void>;
  onDelete: () => void;
}

const META_LABEL_CLASS = 'text-12px leading-18px text-t-tertiary';
const META_VALUE_CLASS = 'text-12px leading-18px text-t-secondary tabular-nums';

/**
 * The full memory, edited in place in the right-hand pane — it stays open while
 * the user keeps scanning the list, which is exactly what the old edit Modal
 * could not do. Kind is read-only here: reclassifying is a batch operation, so
 * the pane does not duplicate it as a per-row control.
 */
const MemoryDetailPane: React.FC<MemoryDetailPaneProps> = ({
  memory,
  onSave,
  onTogglePin,
  onToggleArchive,
  onDelete,
}) => {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(memory.content);
  const [saving, setSaving] = useState(false);

  // Follow the selected row; a live update from nomi replaces the draft only
  // when the user is looking at a different memory.
  useEffect(() => {
    setDraft(memory.content);
  }, [memory.memory_id, memory.content]);

  const archived = memory.status === 'archived';
  const dirty = draft.trim().length > 0 && draft.trim() !== memory.content;
  // No ownership read-out here on purpose: every memory belongs to the companion
  // whose workspace this is, so "who else is affected" is not a question any more.

  const save = async () => {
    if (!dirty || saving) return;
    setSaving(true);
    try {
      await onSave(draft.trim());
      Message.success(t('nomi.memories.saved', { defaultValue: '记忆已保存' }));
    } catch (e) {
      Message.error(String(e));
    } finally {
      setSaving(false);
    }
  };

  const guard = (run: () => Promise<void>) => () => {
    void run().catch((e) => Message.error(String(e)));
  };

  return (
    <div className='flex flex-col gap-16px'>
      <div className='flex flex-col gap-8px'>
        <Input.TextArea
          value={draft}
          onChange={setDraft}
          autoSize={{ minRows: 6, maxRows: 16 }}
          className='!rd-8px text-13px leading-20px'
        />
        <div className='flex items-center justify-between gap-10px'>
          <span className='text-11px leading-16px text-t-tertiary'>
            {t('nomi.memories.editHint', {
              defaultValue: '编辑只影响新对话与实时检索；已经打开的对话仍沿用开聊时的记忆快照。',
            })}
          </span>
          <div
            role='button'
            tabIndex={dirty ? 0 : -1}
            aria-disabled={!dirty || saving}
            onClick={() => void save()}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                void save();
              }
            }}
            className={[
              'inline-flex shrink-0 select-none items-center rd-full px-18px py-9px text-13px font-700 leading-none transition-colors',
              dirty && !saving
                ? 'cursor-pointer bg-[rgba(var(--primary-6),0.12)] text-[var(--color-text-1)] shadow-[0_6px_18px_rgba(var(--primary-6),0.14)] hover:bg-[rgba(var(--primary-6),0.18)]'
                : 'cursor-not-allowed bg-fill-2 text-t-tertiary',
            ].join(' ')}
          >
            {t('nomi.memory.save', { defaultValue: '保存' })}
          </div>
        </div>
      </div>

      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.memory.pinRow', { defaultValue: '置顶' })}
          description={t('nomi.memory.pinRowHint', { defaultValue: '置顶的记忆不会随时间衰减，始终优先被检索到。' })}
          controls={
            <Switch
              className='compact-dark-switch'
              checked={memory.pinned}
              onChange={(checked) => guard(() => onTogglePin(checked))()}
            />
          }
        />
        <NomiSettingRow
          title={archived ? t('nomi.memory.archivedRow', { defaultValue: '已归档' }) : t('nomi.memories.archive', { defaultValue: '归档' })}
          description={t('nomi.memory.archiveRowHint', {
            defaultValue: '归档后不再参与检索，但保留痕迹，可随时恢复。',
          })}
          controls={
            <Switch
              className='compact-dark-switch'
              checked={archived}
              onChange={(checked) => guard(() => onToggleArchive(checked))()}
            />
          }
        />
        <NomiSettingRow
          title={t('nomi.memories.delete', { defaultValue: '删除' })}
          description={t('nomi.memory.deleteRowHint', { defaultValue: '永久删除这条记忆，无法恢复。' })}
          controls={
            <div
              role='button'
              tabIndex={0}
              onClick={onDelete}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  onDelete();
                }
              }}
              className='inline-flex cursor-pointer select-none items-center rd-8px px-10px py-5px text-13px text-[rgb(var(--danger-6))] transition-colors hover:bg-[rgba(var(--danger-6),0.1)]'
            >
              {t('nomi.memories.delete', { defaultValue: '删除' })}
            </div>
          }
        />
      </NomiSettingList>

      <div className='flex flex-col gap-6px'>
        <div className='flex items-center justify-between gap-10px'>
          <span className={META_LABEL_CLASS}>{t('nomi.memory.metaKind', { defaultValue: '分类' })}</span>
          <span className='flex items-center gap-6px text-12px leading-18px text-t-secondary'>
            <span aria-hidden className='h-6px w-6px shrink-0 rd-full' style={{ background: MEMORY_KIND_DOT[memory.kind] }} />
            {t(`nomi.kinds.${memory.kind}`)}
          </span>
        </div>
        <div className='flex items-center justify-between gap-10px'>
          <span className={META_LABEL_CLASS}>{t('nomi.memory.metaImportance', { defaultValue: '重要度' })}</span>
          <span className={META_VALUE_CLASS}>{memory.importance.toFixed(2)}</span>
        </div>
        <div className='flex items-center justify-between gap-10px'>
          <span className={META_LABEL_CLASS}>{t('nomi.memories.strength', { defaultValue: '强度' })}</span>
          <span className={META_VALUE_CLASS}>{`${Math.round(memory.strength * 100)}%`}</span>
        </div>
        <div className='flex items-center justify-between gap-10px'>
          <span className={META_LABEL_CLASS}>{t('nomi.memory.metaSource', { defaultValue: '来源' })}</span>
          <span className='text-12px leading-18px text-t-secondary'>
            {t(`nomi.memories.source_${memory.source}`, { defaultValue: memory.source })}
          </span>
        </div>
        <div className='flex items-center justify-between gap-10px'>
          <span className={META_LABEL_CLASS}>{t('nomi.memory.metaCreatedAt', { defaultValue: '创建时间' })}</span>
          <span className={META_VALUE_CLASS}>{formatMemoryTime(memory.created_at)}</span>
        </div>
        <div className='flex items-center justify-between gap-10px'>
          <span className={META_LABEL_CLASS}>{t('nomi.memory.metaUpdatedAt', { defaultValue: '更新时间' })}</span>
          <span className={META_VALUE_CLASS}>{formatMemoryTime(memory.updated_at)}</span>
        </div>
      </div>
    </div>
  );
};

export default MemoryDetailPane;
