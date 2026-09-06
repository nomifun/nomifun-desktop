/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Drawer, Input } from '@arco-design/web-react';
import { CheckSmall, Close, Search } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { resolveSkillDisplay, type LocalizableSkill } from '@/renderer/pages/settings/skill/skillDisplay';
import styles from '../index.module.css';

export type GuidSkillItem = LocalizableSkill & {
  isAuto: boolean;
};

export type GuidSkillsDrawerProps = {
  visible: boolean;
  onClose: () => void;
  skills: GuidSkillItem[];
  enabledSkills: string[];
  disabledBuiltinSkills: string[];
  onToggleSkill: (name: string, isAuto: boolean) => void;
  localeKey: string;
};

const drawerWidth = (): number => {
  const viewportWidth = window.innerWidth || 1024;
  return Math.min(720, Math.max(360, Math.floor(viewportWidth * 0.52)), Math.max(280, viewportWidth - 24));
};

/**
 * Session-scoped Skill chooser. It deliberately has no AgentPreset catalog,
 * tag vocabulary, or authoring controls.
 */
const GuidSkillsDrawer: React.FC<GuidSkillsDrawerProps> = ({
  visible,
  onClose,
  skills,
  enabledSkills,
  disabledBuiltinSkills,
  onToggleSkill,
  localeKey,
}) => {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const [width, setWidth] = useState(drawerWidth);

  useEffect(() => {
    if (visible) setQuery('');
  }, [visible]);

  useEffect(() => {
    const onResize = () => setWidth(drawerWidth());
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  const filteredSkills = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return skills;
    return skills.filter((skill) => {
      const display = resolveSkillDisplay(skill, localeKey);
      return `${skill.name} ${skill.description ?? ''} ${display.name} ${display.description}`
        .toLowerCase()
        .includes(normalized);
    });
  }, [localeKey, query, skills]);

  const isChecked = (skill: GuidSkillItem): boolean =>
    skill.isAuto ? !disabledBuiltinSkills.includes(skill.name) : enabledSkills.includes(skill.name);

  const selectedCount = useMemo(
    () => skills.filter((skill) => isChecked(skill)).length,
    [disabledBuiltinSkills, enabledSkills, skills]
  );

  return (
    <Drawer
      closable={false}
      visible={visible}
      placement='right'
      width={width}
      zIndex={1200}
      getPopupContainer={() => document.body}
      autoFocus={false}
      onCancel={onClose}
      footer={null}
      headerStyle={{ display: 'none' }}
      bodyStyle={{ padding: 0, height: '100%' }}
    >
      <div className={styles.drawerSurface}>
        <div className={styles.drawerTopbar}>
          <div>
            <strong className='text-15px text-t-primary'>
              {t('guid.drawer.skillsTab', { defaultValue: 'Skills' })}
            </strong>
            <span className='ml-8px text-12px text-t-tertiary'>
              {t('guid.drawer.selectedCount', {
                count: selectedCount,
                defaultValue: '{{count}} selected for this Session',
              })}
            </span>
          </div>
          <button
            type='button'
            className={styles.drawerCloseButton}
            onClick={onClose}
            aria-label={t('common.close', { defaultValue: 'Close' })}
          >
            <Close theme='outline' size={16} strokeWidth={3} />
          </button>
        </div>

        <div className={styles.drawerSearchPanel}>
          <Input
            prefix={<Search theme='outline' size={15} />}
            placeholder={t('guid.drawer.searchSkill', { defaultValue: 'Search Skills…' })}
            value={query}
            onChange={setQuery}
            allowClear
            className={styles.drawerSearchInput}
          />
        </div>

        <div className={styles.drawerResultMeta}>
          <span>
            <strong>{filteredSkills.length}</strong>{' '}
            {t('guid.drawer.skillCount', { defaultValue: 'Skills' })}
          </span>
        </div>

        <div className={styles.drawerList}>
          {filteredSkills.length > 0 ? (
            filteredSkills.map((skill) => {
              const display = resolveSkillDisplay(skill, localeKey);
              const checked = isChecked(skill);
              const initials =
                display.name.replace(/[^a-zA-Z]/g, '').slice(0, 2).toUpperCase() ||
                display.name.slice(0, 2).toUpperCase();
              return (
                <button
                  type='button'
                  key={skill.name}
                  className={[
                    styles.drawerCard,
                    styles.drawerSkillCard,
                    checked ? styles.drawerCardSelected : '',
                  ]
                    .filter(Boolean)
                    .join(' ')}
                  onClick={() => onToggleSkill(skill.name, skill.isAuto)}
                  aria-pressed={checked}
                >
                  <span className={styles.drawerIconTile}>{initials}</span>
                  <span className={styles.drawerCardBody}>
                    <span className={[styles.drawerCardTitleRow, styles.drawerSkillTitleRow].join(' ')}>
                      <strong className={styles.drawerCardTitle} title={display.name}>
                        {display.name}
                      </strong>
                      <span className={[styles.drawerBadge, styles.drawerBadgeMuted].join(' ')}>
                        {skill.isAuto
                          ? t('guid.drawer.autoInject', { defaultValue: 'Auto' })
                          : t('guid.drawer.sourceCustom', { defaultValue: 'Available' })}
                      </span>
                    </span>
                    <span
                      className={[styles.drawerDescription, styles.drawerSkillDescription].join(' ')}
                      title={display.description}
                    >
                      {display.description}
                    </span>
                  </span>
                  <span
                    className={[
                      styles.drawerCardStatus,
                      checked ? styles.drawerCardStatusSelected : '',
                    ]
                      .filter(Boolean)
                      .join(' ')}
                    aria-hidden='true'
                  >
                    {checked && <CheckSmall theme='filled' size={13} fill='currentColor' />}
                  </span>
                </button>
              );
            })
          ) : (
            <div className={styles.drawerEmptyState}>
              {t('guid.drawer.skillNoMatch', { defaultValue: 'No Skills match your search.' })}
            </div>
          )}
        </div>

        <div className={styles.drawerFooter}>
          <span className={styles.drawerFooterHint}>
            {t('guid.drawer.selectedCount', {
              count: selectedCount,
              defaultValue: '{{count}} selected for this Session',
            })}
          </span>
          <button type='button' className={styles.drawerPrimaryButton} onClick={onClose}>
            {t('guid.drawer.applySkills', { defaultValue: 'Done' })}
          </button>
        </div>
      </div>
    </Drawer>
  );
};

export default GuidSkillsDrawer;
