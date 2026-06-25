/**
 * DrawerSkillCard — Multi-select skill card for SummonDrawer.
 * Displays initials avatar, name, source/auto-inject badge, description,
 * tag chips, and a right-side checkbox.
 */
import type { SkillInfo } from '@/renderer/pages/settings/AssistantSettings/types';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { CheckSmall } from '@icon-park/react';
import styles from '../index.module.css';

export type DrawerSkillCardProps = {
  skill: SkillInfo;
  checked: boolean;
  isAuto: boolean;
  onToggle: (name: string, isAuto: boolean) => void;
};

const DrawerSkillCard: React.FC<DrawerSkillCardProps> = ({ skill, checked, isAuto, onToggle }) => {
  const { t } = useTranslation();

  // Generate initials from skill name (first 2 uppercase chars)
  const initials = skill.name
    .replace(/[^a-zA-Z]/g, '')
    .slice(0, 2)
    .toUpperCase() || skill.name.slice(0, 2).toUpperCase();

  const sourceLabel = skill.is_custom
    ? t('guid.drawer.sourceCustom', { defaultValue: '自定义' })
    : t('guid.drawer.sourceBuiltin', { defaultValue: '内置' });
  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      onToggle(skill.name, isAuto);
    }
  };

  return (
    <div
      role='checkbox'
      tabIndex={0}
      aria-checked={checked}
      className={[
        styles.drawerCard,
        checked ? styles.drawerCardSelected : '',
      ].filter(Boolean).join(' ')}
      onClick={() => onToggle(skill.name, isAuto)}
      onKeyDown={handleKeyDown}
    >
      {/* Checkbox indicator */}
      <span
        className={[
          styles.drawerCardStatus,
          checked ? styles.drawerCardStatusSelected : '',
        ].filter(Boolean).join(' ')}
        aria-hidden='true'
      >
        {checked && <CheckSmall theme='filled' size={13} fill='currentColor' />}
      </span>

      {/* Avatar initials */}
      <div className={styles.drawerIconTile}>
        {initials}
      </div>

      {/* Body */}
      <div className={styles.drawerCardBody}>
        <div className={styles.drawerCardTitleRow}>
          <h4 className={styles.drawerCardTitle}>{skill.name}</h4>
          <span className={[styles.drawerBadge, styles.drawerBadgeMuted].join(' ')}>
            {sourceLabel}
          </span>
          {isAuto && (
            <span className={[styles.drawerBadge, styles.drawerBadgeSuccess].join(' ')}>
              {t('guid.drawer.autoInject', { defaultValue: '自动注入' })}
            </span>
          )}
        </div>

        <p className={styles.drawerDescription}>{skill.description}</p>

        {/* Tag chips */}
        {(skill.audience_tags?.length || skill.scenario_tags?.length) ? (
          <div className={styles.drawerMetaRow}>
            {skill.audience_tags?.slice(0, 2).map((tag) => (
              <span key={tag} className={styles.drawerTagChip}>
                {tag}
              </span>
            ))}
            {skill.scenario_tags?.slice(0, 2).map((tag) => (
              <span key={tag} className={styles.drawerTagChip}>
                {tag}
              </span>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
};

export default DrawerSkillCard;
