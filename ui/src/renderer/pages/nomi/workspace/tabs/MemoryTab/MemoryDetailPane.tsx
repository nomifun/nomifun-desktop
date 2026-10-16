/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Input, Message, Switch } from '@arco-design/web-react';
import { Link } from 'react-router-dom';
import type { ICompanionMemory } from '@/common/adapter/ipcBridge';
import { NomiSettingList, NomiSettingRow } from '@/renderer/components/base/NomiSettingLayout';
import { MEMORY_KIND_DOT, formatMemoryTime } from './constants';
import { isMemoryRevisionConflict, memoryMutationErrorKey } from './memoryMutationError';

interface MemoryDetailPaneProps {
  memory: ICompanionMemory;
  onSave: (content: string, observed: ICompanionMemory) => Promise<ICompanionMemory>;
  onTogglePin: (pinned: boolean) => Promise<ICompanionMemory>;
  onToggleArchive: (archived: boolean) => Promise<ICompanionMemory>;
  onDelete: () => void;
  onRefresh: () => Promise<void>;
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
  onRefresh,
}) => {
  const { t } = useTranslation();
  const [editor, setEditor] = useState({ base: memory, draft: memory.content });
  const [saving, setSaving] = useState(false);
  const pending = useRef(false);
  const [conflict, setConflict] = useState(false);
  const { base, draft } = editor;

  // Follow the selected row; a live update from nomi replaces the draft only
  // when the user is looking at a different memory.
  useEffect(() => {
    setEditor(current => {
      if (current.base.memory_id !== memory.memory_id) return { base: memory, draft: memory.content };
      if (memory.revision <= current.base.revision || current.draft.trim() !== current.base.content) return current;
      return { base: memory, draft: memory.content };
    });
  }, [memory]);

  const archived = memory.status === 'archived';
  const dirty = draft.trim().length > 0 && draft.trim() !== base.content;
  const stale = memory.revision > base.revision;
  // No ownership read-out here on purpose: every memory belongs to the companion
  // whose workspace this is, so "who else is affected" is not a question any more.

  const save = async () => {
    if (!dirty || pending.current || stale || conflict) return;
    pending.current = true;
    setSaving(true);
    try {
      const updated = await onSave(draft.trim(), base);
      setEditor({ base: updated, draft: updated.content });
      setConflict(false);
      Message.success(t('nomi.memories.saved', { defaultValue: '记忆已保存' }));
    } catch (e) {
      setConflict(isMemoryRevisionConflict(e));
      Message.error(t(memoryMutationErrorKey(e)));
    } finally {
      setSaving(false);
      pending.current = false;
    }
  };

  const guard = (run: () => Promise<ICompanionMemory>) => async () => {
    if (pending.current || stale || conflict) return;
    pending.current = true;
    setSaving(true);
    try {
      const updated = await run();
      setEditor(current => updated.content === current.base.content && updated.revision > current.base.revision
        ? { ...current, base: updated } : current);
    }
    catch (error) { setConflict(isMemoryRevisionConflict(error)); Message.error(t(memoryMutationErrorKey(error))); }
    finally { pending.current = false; setSaving(false); }
  };

  return (
    <div className='flex flex-col gap-16px'>
      <div className='flex flex-col gap-8px'>
        {(stale || conflict) && <div role='alert' className='flex flex-col gap-6px text-12px text-t-secondary'>
          <span>{t('nomi.memory.conflictDraftKept')}</span>
          {stale && <blockquote className='m-0 whitespace-pre-wrap'>{memory.content}</blockquote>}
          <Button size='mini' onClick={() => {
            if (stale || !dirty) { setEditor({ base: memory, draft: memory.content }); setConflict(false); }
            void onRefresh();
          }}>{t('nomi.memory.loadLatest')}</Button>
        </div>}
        <Input.TextArea
          aria-label={t('nomi.memory.content')}
          value={draft}
          disabled={saving}
          onChange={value => setEditor(current => ({ ...current, draft: value }))}
          autoSize={{ minRows: 6, maxRows: 16 }}
          className='!rd-8px text-13px leading-20px'
        />
        <div className='flex items-center justify-between gap-10px'>
          <span className='text-11px leading-16px text-t-tertiary'>
            {t('nomi.memories.editHint', {
              defaultValue: '编辑只影响新对话与实时检索；已经打开的对话仍沿用开聊时的记忆快照。',
            })}
          </span>
          <Button
            type='primary'
            size='small'
            disabled={!dirty || saving || stale || conflict}
            onClick={() => void save()}
          >
            {t('nomi.memory.save', { defaultValue: '保存' })}
          </Button>
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
              disabled={saving || stale || conflict}
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
              disabled={saving || stale || conflict}
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
              aria-disabled={saving || stale || conflict}
              onClick={() => { if (!saving && !stale && !conflict) onDelete(); }}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  if (!saving && !stale && !conflict) onDelete();
                }
              }}
              className='inline-flex cursor-pointer select-none items-center rd-8px px-10px py-5px text-13px text-danger-6 transition-colors hover:bg-[rgba(var(--danger-6),0.1)]'
            >
              {t('nomi.memories.delete', { defaultValue: '删除' })}
            </div>
          }
        />
      </NomiSettingList>

      <div className='flex flex-col gap-6px'>
        <span className={META_VALUE_CLASS}>{t('nomi.memory.revision', { revision: memory.revision })}</span>
        {memory.source_session_id && memory.companion_id && <Link to={`/nomi?companion=${encodeURIComponent(memory.companion_id)}&session=${encodeURIComponent(memory.source_session_id)}&mode=cohabit`}>
          {t('nomi.memory.sourceConversation')}
        </Link>}
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
