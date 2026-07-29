/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 会话召唤伙伴（spec 2026-07-29 §设计 B5）——工作会话把一位伙伴的技能与勾选
 * 记忆（只读）装进来：
 *
 * - `SummonControl`：SendBox 工具条按钮 + 三步 Drawer（伙伴单选 → 技能复选
 *   （默认全选，去勾 = skill_exclusions）→ 记忆多选（FTS 搜索/kind 过滤，
 *   复用 A 轨道 listMemories 检索面 + 预算字数条））。已召唤时按钮变徽标态，
 *   点开可查看/调整/解除（解除 DELETE，幂等）。
 * - `SummonHeaderBadge`：会话头部的被动徽标（伙伴名），侧边栏条目徽标见
 *   `SessionList/utils/sessionCapabilityItems.tsx`。
 *
 * 召唤/调整/解除要求会话空闲：后端非空闲返回 409，这里转成可读提示。变更
 * 「下一条消息生效」（运行时按 knowledge 绑定同款路径重建）。
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Checkbox, Drawer, Empty, Input, Message, Select, Spin, Tag, Tooltip } from '@arco-design/web-react';
import { EveryUser } from '@icon-park/react';
import useSWR from 'swr';

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { ICompanionMemory, ICompanionMemoryKind, ICompanionWithStatus, ISummonConfig } from '@/common/adapter/ipcBridge';
import type { CompanionId, CompanionMemoryId, ConversationId } from '@/common/types/ids';
import { getConversationOrNull, refreshConversationCache } from '../../utils/conversationCache';

/** Mirror of the backend snapshot budget (`SUMMON_CONTEXT_BUDGET`, 8000 chars). */
export const SUMMON_CONTEXT_BUDGET = 8000;

const MEMORY_KINDS: ICompanionMemoryKind[] = ['profile', 'preference', 'knowledge', 'episode', 'task', 'affective'];

const summonOf = (conversation: { extra?: unknown } | null | undefined): ISummonConfig | null => {
  const extra = conversation?.extra as { summon?: ISummonConfig } | undefined;
  return extra?.summon ?? null;
};

/** Live summon marker of one conversation (shares the page's SWR cache key). */
const useConversationSummon = (conversationId: ConversationId): ISummonConfig | null => {
  const { data } = useSWR(`conversation/${conversationId}`, () => getConversationOrNull(conversationId));
  return summonOf(data);
};

const useCompanionRoster = (): ICompanionWithStatus[] => {
  const { data } = useSWR('companion/roster/summon', () => ipcBridge.companion.listCompanions.invoke(), {
    revalidateOnFocus: false,
  });
  return data ?? [];
};

/** Effective skill set of a companion profile: builtin auto skills minus
 * `disabled_auto`, plus explicitly enabled ones (mirror of the backend's
 * `normalized_effective_skill_names`). */
const useCompanionEffectiveSkills = (companion: ICompanionWithStatus | null): string[] => {
  const { data: autoSkills } = useSWR(
    'skills/builtin-auto/summon',
    () => ipcBridge.fs.listBuiltinAutoSkills.invoke(),
    { revalidateOnFocus: false }
  );
  return useMemo(() => {
    if (!companion) return [];
    const disabled = new Set((companion.skills?.disabled_auto ?? []).map((n) => n.trim()));
    const names = new Set<string>();
    for (const skill of autoSkills ?? []) {
      if (!disabled.has(skill.name)) names.add(skill.name);
    }
    for (const name of companion.skills?.enabled ?? []) {
      const trimmed = name.trim();
      if (trimmed) names.add(trimmed);
    }
    return Array.from(names).sort();
  }, [companion, autoSkills]);
};

const errorToast = (t: (key: string, options?: Record<string, unknown>) => string, error: unknown) => {
  if (isBackendHttpError(error) && error.status === 409) {
    Message.warning(t('conversation.summon.busy'));
    return;
  }
  Message.error(error instanceof Error ? error.message : String(error));
};

/** 会话头部被动徽标：已召唤时显示伙伴名（调整入口在 SendBox 工具条）。 */
export const SummonHeaderBadge: React.FC<{ conversationId: ConversationId }> = ({ conversationId }) => {
  const { t } = useTranslation();
  const summon = useConversationSummon(conversationId);
  const roster = useCompanionRoster();
  if (!summon) return null;
  const name = roster.find((c) => c.companion_id === summon.companion_id)?.name ?? t('conversation.summon.unknownCompanion');
  return (
    <Tooltip content={t('conversation.summon.badgeTooltip', { name })}>
      <Tag
        color='arcoblue'
        size='small'
        icon={<EveryUser theme='outline' size='12' fill='currentColor' />}
        data-testid='summon-header-badge'
      >
        {name}
      </Tag>
    </Tooltip>
  );
};

const SummonControl: React.FC<{ conversationId: ConversationId }> = ({ conversationId }) => {
  const { t } = useTranslation();
  const summon = useConversationSummon(conversationId);
  const roster = useCompanionRoster();
  const [visible, setVisible] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  // Step 1 — companion (single choice).
  const [companionId, setCompanionId] = useState<CompanionId | null>(null);
  const companion = roster.find((c) => c.companion_id === companionId) ?? null;

  // Step 2 — skills: default ALL active checked; unchecked = skill_exclusions.
  const effectiveSkills = useCompanionEffectiveSkills(companion);
  const [excludedSkills, setExcludedSkills] = useState<Set<string>>(new Set());

  // Step 3 — memories: FTS search + kind filter + multi-select with a
  // character-budget meter (re-uses the A-track /api/companion/memories face).
  const [memoryQuery, setMemoryQuery] = useState('');
  const [memoryKind, setMemoryKind] = useState<string>('');
  const [memories, setMemories] = useState<ICompanionMemory[]>([]);
  const [memoriesLoading, setMemoriesLoading] = useState(false);
  const [selectedMemoryIds, setSelectedMemoryIds] = useState<CompanionMemoryId[]>([]);
  const [memoryContents, setMemoryContents] = useState<Map<CompanionMemoryId, string>>(new Map());

  // Prefill from the live summon whenever the drawer opens.
  useEffect(() => {
    if (!visible) return;
    setCompanionId(summon?.companion_id ?? roster[0]?.companion_id ?? null);
    setExcludedSkills(new Set(summon?.skill_exclusions ?? []));
    setSelectedMemoryIds((summon?.memory_ids ?? []) as CompanionMemoryId[]);
    setMemoryQuery('');
    setMemoryKind('');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible]);

  // Memory search scoped to the summoned companion's visibility (shared + its
  // own private memories) — the same scope the read-only recall tool gets.
  useEffect(() => {
    if (!visible || !companionId) return;
    let cancelled = false;
    setMemoriesLoading(true);
    ipcBridge.companion
      .listMemories.invoke({
        q: memoryQuery.trim() || undefined,
        kind: memoryKind || undefined,
        status: 'all',
        scope_companion_id: companionId,
        sort: memoryQuery.trim() ? 'relevance' : 'importance',
        limit: 50,
        offset: 0,
      })
      .then((page) => {
        if (cancelled) return;
        setMemories(page.items);
        setMemoryContents((previous) => {
          const next = new Map(previous);
          for (const memory of page.items) next.set(memory.memory_id, memory.content);
          return next;
        });
      })
      .catch(() => {
        if (!cancelled) setMemories([]);
      })
      .finally(() => {
        if (!cancelled) setMemoriesLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [visible, companionId, memoryQuery, memoryKind]);

  const budgetUsed = useMemo(
    () => selectedMemoryIds.reduce((sum, id) => sum + (memoryContents.get(id)?.length ?? 0), 0),
    [selectedMemoryIds, memoryContents]
  );

  const toggleMemory = (id: CompanionMemoryId) => {
    setSelectedMemoryIds((previous) =>
      previous.includes(id) ? previous.filter((existing) => existing !== id) : [...previous, id]
    );
  };

  const apply = async () => {
    if (!companionId) return;
    setSubmitting(true);
    try {
      await ipcBridge.conversation.setSummon.invoke({
        conversation_id: conversationId,
        companion_id: companionId,
        memory_ids: selectedMemoryIds,
        skill_exclusions: Array.from(excludedSkills),
      });
      Message.success(t('conversation.summon.applied'));
      await refreshConversationCache(conversationId);
      setVisible(false);
    } catch (error) {
      errorToast(t, error);
    } finally {
      setSubmitting(false);
    }
  };

  const release = async () => {
    setSubmitting(true);
    try {
      await ipcBridge.conversation.clearSummon.invoke({ conversation_id: conversationId });
      Message.success(t('conversation.summon.released'));
      await refreshConversationCache(conversationId);
      setVisible(false);
    } catch (error) {
      errorToast(t, error);
    } finally {
      setSubmitting(false);
    }
  };

  const summonedName = summon
    ? (roster.find((c) => c.companion_id === summon.companion_id)?.name ?? t('conversation.summon.unknownCompanion'))
    : null;

  return (
    <>
      <Tooltip
        content={
          summon ? t('conversation.summon.badgeTooltip', { name: summonedName }) : t('conversation.summon.buttonTooltip')
        }
      >
        <Button
          size='small'
          shape='round'
          type={summon ? 'primary' : 'secondary'}
          status={summon ? 'success' : undefined}
          icon={<EveryUser theme='outline' size='14' fill='currentColor' />}
          onClick={() => setVisible(true)}
          data-testid='summon-control-button'
          className='nomi-sendbox-summon-btn'
        >
          <span className='text-12px max-w-90px inline-block overflow-hidden text-ellipsis whitespace-nowrap align-middle'>
            {summon ? summonedName : t('conversation.summon.button')}
          </span>
        </Button>
      </Tooltip>
      <Drawer
        width={430}
        title={t('conversation.summon.title')}
        visible={visible}
        onCancel={() => setVisible(false)}
        footer={
          <div className='flex items-center justify-between w-full'>
            <span>
              {summon && (
                <Button status='danger' loading={submitting} onClick={release} data-testid='summon-release-button'>
                  {t('conversation.summon.release')}
                </Button>
              )}
            </span>
            <span className='flex gap-8px'>
              <Button onClick={() => setVisible(false)}>{t('common.cancel', { defaultValue: 'Cancel' })}</Button>
              <Button type='primary' loading={submitting} disabled={!companionId} onClick={apply} data-testid='summon-apply-button'>
                {summon ? t('conversation.summon.update') : t('conversation.summon.apply')}
              </Button>
            </span>
          </div>
        }
      >
        {/* Step 1 — 伙伴单选 */}
        <div className='text-13px font-500 mb-8px'>{t('conversation.summon.stepCompanion')}</div>
        {roster.length === 0 ? (
          <Empty description={t('conversation.summon.noCompanions')} />
        ) : (
          <div className='flex flex-wrap gap-8px mb-16px'>
            {roster.map((profile) => (
              <div
                key={profile.companion_id}
                onClick={() => {
                  if (profile.companion_id !== companionId) {
                    setCompanionId(profile.companion_id);
                    setExcludedSkills(new Set());
                    setSelectedMemoryIds([]);
                  }
                }}
                className={`cursor-pointer rounded-8px border px-12px py-8px text-13px ${
                  profile.companion_id === companionId
                    ? 'border-[rgb(var(--primary-6))] text-[rgb(var(--primary-6))]'
                    : 'border-[var(--color-border-2)]'
                }`}
                data-testid='summon-companion-card'
              >
                <div className='font-500'>{profile.name}</div>
                <div className='text-11px op-60'>Lv.{profile.status?.level ?? 1}</div>
              </div>
            ))}
          </div>
        )}

        {/* Step 2 — 技能复选（默认全选 active；去勾 = 排除） */}
        <div className='text-13px font-500 mb-8px'>{t('conversation.summon.stepSkills')}</div>
        {effectiveSkills.length === 0 ? (
          <div className='text-12px op-60 mb-16px'>{t('conversation.summon.noSkills')}</div>
        ) : (
          <div className='flex flex-col gap-4px mb-16px max-h-140px overflow-auto'>
            {effectiveSkills.map((name) => (
              <Checkbox
                key={name}
                checked={!excludedSkills.has(name)}
                onChange={(checked) => {
                  setExcludedSkills((previous) => {
                    const next = new Set(previous);
                    if (checked) next.delete(name);
                    else next.add(name);
                    return next;
                  });
                }}
              >
                <span className='text-12px'>{name}</span>
              </Checkbox>
            ))}
          </div>
        )}

        {/* Step 3 — 记忆多选 + 预算字数条 */}
        <div className='text-13px font-500 mb-8px'>{t('conversation.summon.stepMemories')}</div>
        <div className='flex gap-8px mb-8px'>
          <Input.Search
            allowClear
            placeholder={t('conversation.summon.searchMemories')}
            onSearch={(value) => setMemoryQuery(value)}
            onClear={() => setMemoryQuery('')}
            style={{ flex: 1 }}
          />
          <Select allowClear placeholder='kind' style={{ width: 110 }} value={memoryKind || undefined} onChange={(v) => setMemoryKind(v ?? '')}>
            {MEMORY_KINDS.map((kind) => (
              <Select.Option key={kind} value={kind}>
                {kind}
              </Select.Option>
            ))}
          </Select>
        </div>
        <div className='text-12px op-70 mb-8px' data-testid='summon-budget-meter'>
          {t('conversation.summon.budget', { used: budgetUsed, budget: SUMMON_CONTEXT_BUDGET })}
          {budgetUsed > SUMMON_CONTEXT_BUDGET && (
            <span className='text-[rgb(var(--warning-6))] ml-4px'>{t('conversation.summon.budgetOverflow')}</span>
          )}
        </div>
        <Spin loading={memoriesLoading} className='w-full'>
          {memories.length === 0 ? (
            <Empty description={t('conversation.summon.noMemories')} />
          ) : (
            <div className='flex flex-col gap-4px max-h-220px overflow-auto'>
              {memories.map((memory) => (
                <label key={memory.memory_id} className='flex items-start gap-6px cursor-pointer'>
                  <Checkbox
                    checked={selectedMemoryIds.includes(memory.memory_id)}
                    onChange={() => toggleMemory(memory.memory_id)}
                  />
                  <span className='text-12px min-w-0'>
                    <Tag size='small' className='mr-4px'>
                      {memory.kind}
                    </Tag>
                    {memory.status === 'archived' && (
                      <Tag size='small' color='gray' className='mr-4px'>
                        {t('conversation.summon.archived')}
                      </Tag>
                    )}
                    {memory.content}
                  </span>
                </label>
              ))}
            </div>
          )}
        </Spin>
        <div className='text-12px op-60 mt-8px'>{t('conversation.summon.selectedCount', { count: selectedMemoryIds.length })}</div>
      </Drawer>
    </>
  );
};

export default SummonControl;
