/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Trigger } from '@arco-design/web-react';
import { EveryUser, Lightning, Robot } from '@icon-park/react';
import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { resolveSkillDisplay, type LocalizableSkill } from '@/renderer/pages/settings/skill/skillDisplay';
import styles from '../index.module.css';

export type GuidActiveSkill = LocalizableSkill & {
  isAuto?: boolean;
};

export interface ComposerEntryStripProps {
  isPresetAgent: boolean;
  presetLabel?: string;
  presetAvatar?: { kind: 'image' | 'emoji' | 'icon'; value?: string };
  onAdjustSkills: () => void;
  onFree: () => void;
  localeKey: string;
  activeSkillCount?: number;
  activeSkills?: GuidActiveSkill[];
  collaborationPolicyNode?: React.ReactNode;
  /** 召唤伙伴 draft entry — the Guid page wires it only for nomi launches. */
  onSummonCompanion?: () => void;
  /** Name of the drafted companion; the entry shows it as its label. */
  summonedCompanionName?: string | null;
}

/**
 * ComposerEntryStrip — top-edge entry bar inside the chat composer.
 *
 * Both the free-form and preset states begin with the shared collaboration
 * policy, followed by the preset and Skills controls relevant to that state.
 */
const ComposerEntryStrip: React.FC<ComposerEntryStripProps> = ({
  isPresetAgent,
  presetLabel,
  presetAvatar,
  onAdjustSkills,
  onFree,
  localeKey,
  activeSkillCount,
  activeSkills = [],
  collaborationPolicyNode,
  onSummonCompanion,
  summonedCompanionName,
}) => {
  const { t } = useTranslation();
  const [skillsOpen, setSkillsOpen] = useState(false);
  const skillCount = activeSkills.length > 0 ? activeSkills.length : (activeSkillCount ?? 0);
  const skillsLabel =
    skillCount > 0
      ? t('guid.entry.skillsActive', { defaultValue: '使用 Skills · 已启用' })
      : t('guid.entry.skills', { defaultValue: '使用 Skills' });
  const visibleSkills = useMemo(() => activeSkills.slice(0, 4), [activeSkills]);
  const overflowSkillCount = Math.max(0, activeSkills.length - visibleSkills.length);

  // --- Avatar renderer (mirrors GuidPage selectedPresetAvatar pattern) ---
  const renderAvatar = () => {
    if (!presetAvatar) return <Robot theme='outline' size={16} fill='currentColor' />;
    switch (presetAvatar.kind) {
      case 'image':
        return <img src={presetAvatar.value} alt='' className='w-20px h-20px rounded-6px object-contain' />;
      case 'emoji':
        return <span className='text-14px leading-none'>{presetAvatar.value}</span>;
      case 'icon':
      default:
        return <Robot theme='outline' size={16} fill='currentColor' />;
    }
  };

  const skillsPopover =
    activeSkills.length > 0 ? (
      <div className={styles.entrySkillPopover} data-testid='guid-current-skills-popover'>
        <div className={styles.entrySkillPopoverTitleRow}>
          <div className={styles.entrySkillPopoverTitle}>
            {t('guid.skillsPopover.title', {
              defaultValue: '本次会话使用的 Skills',
            })}
          </div>
          <span className={styles.entrySkillPopoverCount}>
            {t('guid.skillsPopover.enabledCount', {
              count: skillCount,
              defaultValue: '已启用 {{count}} 个',
            })}
          </span>
        </div>
        <div className={styles.entrySkillPopoverDesc}>
          {t('guid.skillsPopover.description', {
            defaultValue: '这些 Skills 会随本次会话注入，可在发送前由「使用 Skills」调整。',
          })}
        </div>

        <div className={styles.entrySkillCompactList}>
          {visibleSkills.map((skill) => {
            const display = resolveSkillDisplay(skill, localeKey);
            return (
              <div className={styles.entrySkillCompactRow} key={skill.name}>
                <span className={styles.entrySkillIcon}>
                  <Lightning theme='outline' size={13} strokeWidth={3} />
                </span>
                <div className={styles.entrySkillCompactBody}>
                  <div className={styles.entrySkillCompactNameRow}>
                    <span className={styles.entrySkillCompactName} title={display.name}>
                      {display.name}
                    </span>
                    {skill.isAuto && (
                      <span className={styles.entrySkillSource}>
                        {t('guid.drawer.autoInject', {
                          defaultValue: '自动注入',
                        })}
                      </span>
                    )}
                  </div>
                  {display.description && (
                    <div className={styles.entrySkillCompactDesc} title={display.description}>
                      {display.description}
                    </div>
                  )}
                </div>
              </div>
            );
          })}
          {overflowSkillCount > 0 && (
            <div className={styles.entrySkillOverflow}>
              {t('guid.skillsPopover.overflowCount', {
                count: overflowSkillCount,
                defaultValue: '还有 {{count}} 个 Skills',
              })}
            </div>
          )}
        </div>

        <div className={styles.entrySkillCompactHint}>
          {t('guid.skillsPopover.adjustHint', {
            defaultValue: '点击「使用 Skills」调整本次会话。',
          })}
        </div>
      </div>
    ) : null;

  // --- Skills entry (shared in both states) ---
  const skillsAriaLabel =
    skillCount > 0
      ? t('guid.entry.skillsAdjustAria', {
          count: skillCount,
          defaultValue: '调整本次会话已启用的 {{count}} 个 Skills',
        })
      : t('guid.entry.skills', { defaultValue: '使用 Skills' });
  const skillsButton = (
    <button
      type='button'
      className={`${styles.entryButton} ${styles.entryButtonInteractive}`}
      onClick={onAdjustSkills}
      aria-label={skillsAriaLabel}
    >
      <Lightning theme='outline' size={15} strokeWidth={3} />
      <span className={styles.entryButtonText}>{skillsLabel}</span>
    </button>
  );

  const skillsEntry = skillsPopover ? (
    <span className={styles.entrySkillControl}>
      {skillsButton}
      <Trigger
        popup={() => skillsPopover}
        trigger='click'
        position='top'
        popupVisible={skillsOpen}
        onVisibleChange={setSkillsOpen}
        clickToClose
      >
        <button
          type='button'
          className={`${styles.entryCountBadge} ${styles.entrySkillCountTrigger}`}
          aria-label={t('guid.entry.skillsActiveAria', {
            count: skillCount,
            defaultValue: '查看本次会话已启用的 {{count}} 个 Skills',
          })}
        >
          {skillCount}
        </button>
      </Trigger>
    </span>
  ) : (
    <span className={styles.entrySkillControl}>
      {skillsButton}
      {skillCount > 0 && (
        <span className={styles.entryCountBadge} aria-label={`${skillCount} skills`}>
          {skillCount}
        </span>
      )}
    </span>
  );

  // --- Summon companion entry (optional; nomi launches only) ---
  const summonLabel = summonedCompanionName || t('conversation.summon.button');
  const summonEntry = onSummonCompanion ? (
    <button
      type='button'
      className={`${styles.entryButton} ${styles.entryButtonInteractive}`}
      onClick={onSummonCompanion}
      aria-label={t('conversation.summon.buttonTooltip')}
      data-testid='guid-summon-entry'
    >
      <EveryUser theme='outline' size={15} fill='currentColor' />
      <span className={styles.entryButtonText}>{summonLabel}</span>
    </button>
  ) : null;

  // --- Preset selected state ---
  if (isPresetAgent) {
    const activePresetLabel = presetLabel || t('guid.entry.usePreset', { defaultValue: '使用设定' });

    return (
      <div className={styles.entryStrip}>
        {collaborationPolicyNode}

        {summonEntry}

        {/* Persona token */}
        <span className={`${styles.entryButton} ${styles.entryButtonActive} ${styles.entryPersonaButton}`}>
          <span className={styles.entryAvatar}>{renderAvatar()}</span>
          <span className={styles.entryButtonText}>{activePresetLabel}</span>
          <button
            type='button'
            className={styles.entryDismiss}
            onClick={onFree}
            aria-label={t('guid.entry.backToFree', {
              defaultValue: '自由发挥',
            })}
          >
            ✕
          </button>
        </span>

        {/* Skills */}
        {skillsEntry}

        {/* Right: back to free */}
        <button
          type='button'
          className={styles.entryBackButton}
          onClick={onFree}
          aria-label={t('guid.entry.backToFree', { defaultValue: '自由发挥' })}
        >
          <span>↩</span>
          <span className={styles.entryButtonText}>
            {t('guid.entry.backToFree', { defaultValue: '自由发挥' })}
          </span>
        </button>
      </div>
    );
  }

  // --- Default state ---
  return (
    <div className={styles.entryStrip}>
      {collaborationPolicyNode}

      {summonEntry}

      {/* Skills */}
      {skillsEntry}
    </div>
  );
};

export default ComposerEntryStrip;
