/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Trigger } from '@arco-design/web-react';
import { ApplicationOne, EveryUser, Lightning } from '@icon-park/react';
import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { resolveSkillDisplay, type LocalizableSkill } from '@/renderer/pages/settings/skill/skillDisplay';
import styles from '../index.module.css';

export type GuidActiveSkill = LocalizableSkill & {
  isAuto?: boolean;
};

export interface ComposerEntryStripProps {
  onAdjustSkills: () => void;
  localeKey: string;
  activeSkillCount?: number;
  activeSkills?: GuidActiveSkill[];
  collaborationPolicyNode?: React.ReactNode;
  /** Companion draft entry; only available for Nomi launches. */
  onSummonCompanion?: () => void;
  summonedCompanionName?: string | null;
  /** Mini-app entry. Omit to hide this capability on a surface. */
  onCreateMiniApp?: () => void;
  miniAppActive?: boolean;
  onDismissMiniApp?: () => void;
}

/**
 * The Guid composer only exposes per-session controls here. Agent
 * authoring is intentionally not embedded in the quick-start surface.
 */
const ComposerEntryStrip: React.FC<ComposerEntryStripProps> = ({
  onAdjustSkills,
  localeKey,
  activeSkillCount,
  activeSkills = [],
  collaborationPolicyNode,
  onSummonCompanion,
  summonedCompanionName,
  onCreateMiniApp,
  miniAppActive = false,
  onDismissMiniApp,
}) => {
  const { t } = useTranslation();
  const [skillsOpen, setSkillsOpen] = useState(false);
  const skillCount = activeSkills.length > 0 ? activeSkills.length : (activeSkillCount ?? 0);
  const skillsLabel =
    skillCount > 0
      ? t('guid.entry.skillsActive', { defaultValue: 'Use Skills · Enabled' })
      : t('guid.entry.skills', { defaultValue: 'Use Skills' });
  const visibleSkills = useMemo(() => activeSkills.slice(0, 4), [activeSkills]);
  const overflowSkillCount = Math.max(0, activeSkills.length - visibleSkills.length);

  const skillsPopover =
    activeSkills.length > 0 ? (
      <div className={styles.entrySkillPopover} data-testid='guid-current-skills-popover'>
        <div className={styles.entrySkillPopoverTitleRow}>
          <div className={styles.entrySkillPopoverTitle}>
            {t('guid.skillsPopover.title', { defaultValue: 'Skills for this Session' })}
          </div>
          <span className={styles.entrySkillPopoverCount}>
            {t('guid.skillsPopover.enabledCount', {
              count: skillCount,
              defaultValue: '{{count}} enabled',
            })}
          </span>
        </div>
        <div className={styles.entrySkillPopoverDesc}>
          {t('guid.skillsPopover.description', {
            defaultValue: 'These Skills are applied only to the Session you are about to start.',
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
                        {t('guid.drawer.autoInject', { defaultValue: 'Auto' })}
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
                defaultValue: '{{count}} more Skills',
              })}
            </div>
          )}
        </div>
      </div>
    ) : null;

  const skillsAriaLabel =
    skillCount > 0
      ? t('guid.entry.skillsAdjustAria', {
          count: skillCount,
          defaultValue: 'Adjust {{count}} enabled Skills for this Session',
        })
      : t('guid.entry.skills', { defaultValue: 'Use Skills' });
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
            defaultValue: 'View {{count}} enabled Skills',
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

  const summonEntry = onSummonCompanion ? (
    <button
      type='button'
      className={`${styles.entryButton} ${styles.entryButtonInteractive}`}
      onClick={onSummonCompanion}
      aria-label={t('conversation.summon.buttonTooltip')}
      data-testid='guid-summon-entry'
    >
      <EveryUser theme='outline' size={15} fill='currentColor' />
      <span className={styles.entryButtonText}>
        {summonedCompanionName || t('conversation.summon.button')}
      </span>
    </button>
  ) : null;

  const miniAppEntry = !onCreateMiniApp ? null : miniAppActive ? (
    <span
      className={`${styles.entryButton} ${styles.entryButtonActive} ${styles.entryPersonaButton}`}
      data-testid='guid-miniapp-token'
    >
      <span className={styles.entryAvatar}>
        <ApplicationOne theme='outline' size={16} fill='currentColor' />
      </span>
      <span className={styles.entryButtonText}>{t('miniApps.composer.activeLabel')}</span>
      <button
        type='button'
        className={styles.entryDismiss}
        onClick={onDismissMiniApp}
        aria-label={t('miniApps.composer.dismiss')}
      >
        ×
      </button>
    </span>
  ) : (
    <button
      type='button'
      className={`${styles.entryButton} ${styles.entryButtonInteractive}`}
      onClick={onCreateMiniApp}
      aria-label={t('miniApps.composer.entry')}
      data-testid='guid-miniapp-entry'
    >
      <ApplicationOne theme='outline' size={15} fill='currentColor' />
      <span className={styles.entryButtonText}>{t('miniApps.composer.entry')}</span>
    </button>
  );

  return (
    <div className={styles.entryStrip}>
      {collaborationPolicyNode}
      {summonEntry}
      {miniAppEntry}
      {skillsEntry}
    </div>
  );
};

export default ComposerEntryStrip;
