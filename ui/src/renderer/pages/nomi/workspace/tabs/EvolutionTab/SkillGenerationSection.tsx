/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Switch } from '@arco-design/web-react';
import { CheckOne } from '@icon-park/react';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import InstallWideNote from './InstallWideNote';
import NumberSetting from './NumberSetting';
import type { EvolutionConfigHandle } from './useEvolutionConfig';

const SWITCH_PROPS = { size: 'small' as const, className: 'compact-dark-switch shrink-0' };

/** Confidence threshold applied when the user picks 激进. Never surfaced. */
const AGGRESSIVE_THRESHOLD = 0.85;

type GenerationMode = 'conservative' | 'aggressive';

interface Props {
  config: EvolutionConfigHandle;
}

const ModeCard: React.FC<{
  active: boolean;
  title: string;
  description: string;
  onSelect: () => void;
}> = ({ active, title, description, onSelect }) => (
  <div
    role='button'
    tabIndex={0}
    aria-pressed={active}
    onClick={onSelect}
    onKeyDown={(e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        onSelect();
      }
    }}
    className={classNames(
      'min-w-0 flex-1 cursor-pointer select-none rd-8px border border-solid border-[var(--color-border-2)] px-12px py-9px transition-colors',
      active ? '!bg-primary-1 !text-primary-6' : 'text-t-primary hover:bg-fill-2 active:bg-fill-3'
    )}
  >
    <div className='flex min-w-0 items-center gap-5px text-13px leading-19px font-500'>
      {active && (
        <CheckOne theme='filled' size='13' fill='currentColor' strokeWidth={3} className='line-height-0 shrink-0' />
      )}
      <span className='min-w-0 truncate'>{title}</span>
    </div>
    <div className={classNames('mt-3px text-12px leading-18px', active ? 'opacity-75' : 'text-t-tertiary')}>
      {description}
    </div>
  </div>
);

/**
 * 技能生成配置 — whether repeated multi-step work becomes reusable skills, and
 * how eager the companion may be about it. The eagerness is a two-option choice,
 * not a raw threshold: confidence numbers, pattern counts and decay are
 * implementation detail and stay out of the UI.
 */
const SkillGenerationSection: React.FC<Props> = ({ config }) => {
  const { t } = useTranslation();
  const { evolve, patchEvolve } = config;
  if (!evolve) return null;

  const mode: GenerationMode = evolve.auto_activate ? 'aggressive' : 'conservative';
  const selectMode = (next: GenerationMode) => {
    if (next === mode) return;
    const patch =
      next === 'aggressive'
        ? { auto_activate: true, auto_threshold: AGGRESSIVE_THRESHOLD }
        : { auto_activate: false };
    void patchEvolve(patch).catch((e) => Message.error(String(e)));
  };

  return (
    <NomiSettingSection
      title={t('nomi.evolution.skillTitle', { defaultValue: '技能生成配置' })}
      description={
        <>
          {t('nomi.evolution.skillDesc', {
            defaultValue: '把你反复重复的多步操作沉淀成可复用技能，使用上面的学习模型。',
          })}
          {config.installWide && (
            <InstallWideNote
              text={
                config.ownsGeneratedSkills
                  ? undefined
                  : t('nomi.evolution.skillNotOwnerNote', {
                      defaultValue:
                        '这组设置目前对所有伙伴共同生效；自动生成的技能会归到默认伙伴名下，不会出现在这个伙伴的技能页。',
                    })
              }
            />
          )}
        </>
      }
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.evolve.enabled', { defaultValue: '开启技能进化' })}
          description={t('nomi.evolution.skillEnabledDesc', {
            defaultValue: '关闭后不再生成新技能，已有技能仍然可用。',
          })}
          controls={
            <Switch
              {...SWITCH_PROPS}
              checked={evolve.enabled}
              onChange={(checked) => {
                void patchEvolve({ enabled: checked }).catch((e) => Message.error(String(e)));
              }}
            />
          }
        />
        <NomiSettingRow
          title={t('nomi.evolution.preferenceTitle', { defaultValue: '生成偏好' })}
          description={t('nomi.evolution.preferenceDesc', {
            defaultValue: '决定新技能是先等你过目，还是把握较大时直接启用。',
          })}
          footer={
            <div className='flex gap-8px max-[760px]:flex-col'>
              <ModeCard
                active={mode === 'conservative'}
                title={t('nomi.evolution.preferenceConservative', { defaultValue: '保守' })}
                description={t('nomi.evolution.preferenceConservativeDesc', {
                  defaultValue: '新技能一律存为草稿，由你在技能页确认后才启用。',
                })}
                onSelect={() => selectMode('conservative')}
              />
              <ModeCard
                active={mode === 'aggressive'}
                title={t('nomi.evolution.preferenceAggressive', { defaultValue: '激进' })}
                description={t('nomi.evolution.preferenceAggressiveDesc', {
                  defaultValue: '把握较大的新技能直接启用，其余仍存为草稿（随时可在技能页停用）。',
                })}
                onSelect={() => selectMode('aggressive')}
              />
            </div>
          }
        />
        <NomiSettingRow
          title={t('nomi.evolution.minSessions', { defaultValue: '最少出现会话数' })}
          description={t('nomi.evolution.minSessionsDesc', {
            defaultValue: '同一套操作至少在这么多个会话里出现过，才会被沉淀成技能。',
          })}
          controls={
            <NumberSetting
              min={1}
              max={10}
              value={evolve.min_distinct_sessions}
              onCommit={(min_distinct_sessions) => patchEvolve({ min_distinct_sessions })}
              suffix={t('nomi.evolution.sessionsUnit', { defaultValue: '个会话' })}
            />
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default SkillGenerationSection;
