/**
 * DrawerAssistantCard — Single-select assistant card for SummonDrawer.
 * Displays avatar, name, source badge, description, engine/model capsule, tag chips,
 * radio indicator (top-right), and hover "召唤 →" CTA.
 */
import type { Assistant } from '@/common/types/agent/assistantTypes';
import React from 'react';
import { useTranslation } from 'react-i18next';
import AssistantAvatar from '@/renderer/pages/settings/AssistantSettings/AssistantAvatar';
import { CheckSmall, Right } from '@icon-park/react';
import styles from '../index.module.css';

export type DrawerAssistantCardProps = {
  assistant: Assistant;
  selected: boolean;
  localeKey: string;
  avatarImageMap: Record<string, string>;
  onSelect: (assistantId: string) => void;
};

const DrawerAssistantCard: React.FC<DrawerAssistantCardProps> = ({
  assistant,
  selected,
  localeKey,
  avatarImageMap,
  onSelect,
}) => {
  const { t } = useTranslation();
  const name = assistant.name_i18n?.[localeKey] || assistant.name_i18n?.['en-US'] || assistant.name;
  const description =
    assistant.description_i18n?.[localeKey] || assistant.description_i18n?.['en-US'] || assistant.description || '';
  const isCustom = assistant.source === 'user';
  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      onSelect(assistant.id);
    }
  };

  return (
    <div
      role='button'
      tabIndex={0}
      className={[
        styles.drawerCard,
        selected ? styles.drawerCardSelected : '',
      ].filter(Boolean).join(' ')}
      onClick={() => onSelect(assistant.id)}
      onKeyDown={handleKeyDown}
    >
      {/* Radio indicator */}
      <span
        className={[
          styles.drawerCardStatus,
          selected ? styles.drawerCardStatusSelected : '',
        ].filter(Boolean).join(' ')}
        aria-hidden='true'
      >
        {selected && <CheckSmall theme='filled' size={12} fill='currentColor' />}
      </span>

      {/* Avatar */}
      <div className={styles.drawerIconTile}>
        <AssistantAvatar assistant={assistant} size={40} avatarImageMap={avatarImageMap} />
      </div>

      {/* Body */}
      <div className={styles.drawerCardBody}>
        <div className={styles.drawerCardTitleRow}>
          <h4 className={styles.drawerCardTitle}>{name}</h4>
          <span
            className={[
              styles.drawerBadge,
              isCustom ? styles.drawerBadgePrimary : styles.drawerBadgeMuted,
            ].filter(Boolean).join(' ')}
          >
            {isCustom ? t('guid.drawer.sourceCustom', { defaultValue: '自定义' }) : t('guid.drawer.sourceBuiltin', { defaultValue: '内置' })}
          </span>
        </div>

        <p className={styles.drawerDescription}>{description}</p>

        {/* Meta row: engine capsule + tag chips */}
        <div className={styles.drawerMetaRow}>
          {assistant.models?.[0] && (
            <span className={styles.drawerEngineBadge}>
              <span className={styles.drawerEngineGlyph}>
                {assistant.preset_agent_type?.[0]?.toUpperCase() || '◆'}
              </span>
              {assistant.models[0]}
            </span>
          )}
          {assistant.audience_tags?.slice(0, 2).map((tag) => (
            <span key={tag} className={styles.drawerTagChip}>
              {tag}
            </span>
          ))}
          {assistant.scenario_tags?.slice(0, 2).map((tag) => (
            <span key={tag} className={styles.drawerTagChip}>
              {tag}
            </span>
          ))}
        </div>
      </div>

      {/* Hover summon CTA */}
      <span
        className={styles.drawerCallAction}
        aria-hidden='true'
      >
        {t('guid.drawer.summon', { defaultValue: '召唤' })}
        <Right theme='outline' size={12} strokeWidth={3} />
      </span>
    </div>
  );
};

export default DrawerAssistantCard;
