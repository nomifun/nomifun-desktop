/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Checkbox, Input, Message, Spin } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import type { ICompanionMemoryKind, ICompanionMemoryMergeGroup } from '@/common/adapter/ipcBridge';
import type { CompanionId, CompanionMemoryId } from '@/common/types/ids';
import { MEMORY_KINDS } from './constants';

/** Per-group editable state: which members merge, into what text, under what kind. */
interface MergeDraft {
  ids: CompanionMemoryId[];
  content: string;
  kind: ICompanionMemoryKind;
}

interface MergeAssistantPaneProps {
  companionId: CompanionId;
  /** Refresh the list after a merge lands. */
  onMerged: () => void;
}

/**
 * Duplicate-merge assistant, hosted in the detail pane instead of its own Drawer.
 *
 * The dry run is scoped to `companionId` in the STORE: the response only ever
 * contains memories this companion can see, so there is nothing to filter here —
 * and, more to the point, another companion's memory text never reaches this
 * surface at all. Groups are owner-buckets by construction (a legacy row not yet
 * re-homed has no owner and is visible to everyone until the boot migration
 * assigns it one).
 */
const MergeAssistantPane: React.FC<MergeAssistantPaneProps> = ({ companionId, onMerged }) => {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(true);
  const [groups, setGroups] = useState<ICompanionMemoryMergeGroup[]>([]);
  const [drafts, setDrafts] = useState<MergeDraft[]>([]);

  /**
   * The dry run scans this companion's whole active layer and can outlive the
   * pane, so the caller passes a liveness probe and a closed pane is never
   * written to.
   */
  const load = useCallback(
    async (isAlive: () => boolean) => {
      setLoading(true);
      try {
        const groups = await ipcBridge.companion.memoryMergeSuggestions.invoke({
          scope_companion_id: companionId,
        });
        if (!isAlive()) return;
        setGroups(groups);
        setDrafts(
          groups.map((group) => ({
            ids: group.memories.map((m) => m.memory_id),
            // Pre-fill with the longest member; the user edits before confirming.
            content: group.memories.reduce((best, m) => (m.content.length > best.length ? m.content : best), ''),
            kind: group.memories[0]?.kind ?? 'knowledge',
          }))
        );
      } catch (e) {
        if (!isAlive()) return;
        Message.error(String(e));
        setGroups([]);
        setDrafts([]);
      } finally {
        if (isAlive()) setLoading(false);
      }
    },
    [companionId]
  );

  useEffect(() => {
    let alive = true;
    void load(() => alive);
    return () => {
      alive = false;
    };
  }, [load]);

  const patchDraft = useCallback((index: number, patch: Partial<MergeDraft>) => {
    setDrafts((prev) => prev.map((draft, i) => (i === index ? { ...draft, ...patch } : draft)));
  }, []);

  const submit = useCallback(
    async (index: number) => {
      const draft = drafts[index];
      if (!draft || draft.ids.length < 2 || !draft.content.trim()) return;
      try {
        await ipcBridge.companion.mergeMemories.invoke({
          group: draft.ids,
          merged_content: draft.content.trim(),
          kind: draft.kind,
          scope_companion_id: companionId,
        });
        Message.success(t('nomi.memories.merged', { defaultValue: '已合并' }));
        setGroups((prev) => prev.filter((_, i) => i !== index));
        setDrafts((prev) => prev.filter((_, i) => i !== index));
        onMerged();
      } catch (e) {
        Message.error(String(e));
      }
    },
    [companionId, drafts, onMerged, t]
  );

  if (loading) {
    return (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    );
  }

  if (groups.length === 0) {
    return (
      <div className='py-40px text-center text-13px leading-20px text-t-tertiary'>
        {t('nomi.memories.mergeEmpty', { defaultValue: '没有发现疑似重复的记忆。' })}
      </div>
    );
  }

  return (
    <div className='flex flex-col gap-14px'>
      <div className='text-12px leading-18px text-t-tertiary'>
        {t('nomi.memories.mergeHint', {
          defaultValue: '每组勾选要合并的条目，编辑合并后的内容并确认。被合并的原条目会归档留痕（不删除）。',
        })}
      </div>
      {groups.map((group, index) => {
        const draft = drafts[index];
        if (!draft) return null;
        const ready = draft.ids.length >= 2 && draft.content.trim().length > 0;
        return (
          <div
            key={group.memories[0]?.memory_id ?? index}
            className='flex flex-col gap-8px rd-12px border border-solid border-[var(--color-border-2)] p-12px'
          >
            <div className='flex flex-col gap-6px'>
              {group.memories.map((m) => (
                <Checkbox
                  key={m.memory_id}
                  checked={draft.ids.includes(m.memory_id)}
                  onChange={() =>
                    patchDraft(index, {
                      ids: draft.ids.includes(m.memory_id)
                        ? draft.ids.filter((id) => id !== m.memory_id)
                        : [...draft.ids, m.memory_id],
                    })
                  }
                >
                  <span className='text-13px leading-20px break-words'>{m.content}</span>
                </Checkbox>
              ))}
            </div>

            <div className='flex items-center gap-8px'>
              <span className='text-12px leading-18px text-t-tertiary'>
                {t('nomi.memories.mergeContentLabel', { defaultValue: '合并后的内容' })}
              </span>
              <NomiSelect
                contentFit
                contentMaxWidth={130}
                size='small'
                value={draft.kind}
                onChange={(value: ICompanionMemoryKind) => patchDraft(index, { kind: value })}
              >
                {MEMORY_KINDS.map((item) => (
                  <NomiSelect.Option key={item} value={item}>
                    {t(`nomi.kinds.${item}`)}
                  </NomiSelect.Option>
                ))}
              </NomiSelect>
            </div>

            <Input.TextArea
              value={draft.content}
              onChange={(value: string) => patchDraft(index, { content: value })}
              autoSize={{ minRows: 3, maxRows: 10 }}
              className='!rd-8px text-13px leading-20px'
            />

            <div className='flex justify-end'>
              <div
                role='button'
                tabIndex={ready ? 0 : -1}
                aria-disabled={!ready}
                onClick={() => void submit(index)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    void submit(index);
                  }
                }}
                className={[
                  'inline-flex select-none items-center rd-full px-14px py-7px text-12px font-700 leading-none transition-colors',
                  ready
                    ? 'cursor-pointer bg-[rgba(var(--primary-6),0.12)] text-[var(--color-text-1)] hover:bg-[rgba(var(--primary-6),0.18)]'
                    : 'cursor-not-allowed bg-fill-2 text-t-tertiary',
                ].join(' ')}
              >
                {t('nomi.memories.mergeSubmit', { defaultValue: '合并所选' })}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
};

export default MergeAssistantPane;
