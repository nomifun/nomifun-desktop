/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Empty, Input, Modal, Switch } from '@arco-design/web-react';
import type { ICompanionSkillConfig } from '@/common/adapter/ipcBridge';
import { NomiSettingList, NomiSettingRow } from '@/renderer/components/base/NomiSettingLayout';
import { resolveSkillDisplay } from '@/renderer/pages/settings/skill/skillDisplay';
import { SkillMissingBadge } from './SkillBadges';
import { isSkillGranted, type CatalogSkillInfo } from './unify';

interface CatalogPickerModalProps {
  visible: boolean;
  onClose: () => void;
  catalog: readonly CatalogSkillInfo[];
  autoNames: ReadonlySet<string>;
  config: ICompanionSkillConfig;
  localeKey: string;
  /** Name whose grant patch is in flight. */
  busyName: string | null;
  /** No grant can be changed right now (a patch is in flight / no profile yet). */
  disabled: boolean;
  onToggle: (name: string, granted: boolean) => void;
}

/**
 * The catalog picker. Granting is a *browse* action over a library that has
 * nothing to do with this companion until a switch flips, so it stays a modal:
 * the unified list behind it only ever shows skills the companion actually has.
 */
const CatalogPickerModal: React.FC<CatalogPickerModalProps> = ({
  visible,
  onClose,
  catalog,
  autoNames,
  config,
  localeKey,
  busyName,
  disabled,
  onToggle,
}) => {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');

  const rows = useMemo(() => {
    const known = new Set(catalog.map((skill) => skill.name));
    // A config may still reference an uninstalled Skill; surface it so a stale
    // grant can be cleared from the same place it was made.
    const missing: CatalogSkillInfo[] = [...config.enabled, ...config.disabled_auto]
      .filter((name, index, all) => !known.has(name) && all.indexOf(name) === index)
      .map((name) => ({ name, description: '', location: '', source: 'custom' }));
    const all = [...catalog, ...missing].map((skill) => ({
      skill,
      display: resolveSkillDisplay(skill, localeKey),
    }));
    const needle = query.trim().toLocaleLowerCase();
    const matched = needle
      ? all.filter(
          ({ skill, display }) =>
            display.name.toLocaleLowerCase().includes(needle) ||
            skill.name.toLocaleLowerCase().includes(needle) ||
            display.description.toLocaleLowerCase().includes(needle)
        )
      : all;
    return matched.sort((a, b) => a.display.name.localeCompare(b.display.name));
  }, [catalog, config.disabled_auto, config.enabled, localeKey, query]);

  return (
    <Modal
      title={t('nomi.skills.catalogTitle', { defaultValue: '能力库' })}
      visible={visible}
      onCancel={onClose}
      footer={null}
      style={{ width: 680 }}
    >
      <div className='flex flex-col gap-12px'>
        <div className='text-12px leading-18px text-t-tertiary'>
          {t('nomi.skills.catalogHint', {
            defaultValue: '从技能库里挑选要授予这个伙伴的能力，改动会在下一条消息生效。',
          })}
        </div>
        <Input.Search
          allowClear
          value={query}
          onChange={setQuery}
          placeholder={t('nomi.skills.configSearch', { defaultValue: '搜索 Skill' })}
        />
        <div className='max-h-420px overflow-y-auto'>
          {rows.length === 0 ? (
            <Empty description={t('nomi.skills.configEmpty', { defaultValue: '没有可配置的 Skill' })} />
          ) : (
            <NomiSettingList>
              {rows.map(({ skill, display }) => (
                <NomiSettingRow
                  key={skill.name}
                  className='py-9px'
                  title={
                    <div className='flex min-w-0 items-center gap-6px'>
                      <span className='min-w-0 truncate'>{display.name}</span>
                      {!skill.location && <SkillMissingBadge />}
                    </div>
                  }
                  descriptionClassName='line-clamp-2'
                  description={
                    display.description ||
                    t('nomi.skills.noDescription', { defaultValue: '这个技能还没有描述' })
                  }
                  controls={
                    <Switch
                      size='small'
                      className='compact-dark-switch'
                      aria-label={display.name}
                      checked={isSkillGranted(config, autoNames, skill.name)}
                      loading={busyName === skill.name}
                      disabled={disabled && busyName !== skill.name}
                      onChange={(checked) => onToggle(skill.name, checked)}
                    />
                  }
                />
              ))}
            </NomiSettingList>
          )}
        </div>
      </div>
    </Modal>
  );
};

export default CatalogPickerModal;
