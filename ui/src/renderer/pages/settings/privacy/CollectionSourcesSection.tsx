/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Switch, Tag } from '@arco-design/web-react';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import {
  COLLECTION_SOURCE_KEYS,
  SOURCE_SENSITIVITY,
  type CollectSettingsHandle,
} from './useCollectSettings';

const SENSITIVITY_COLOR = { low: 'green', medium: 'orange', high: 'red' } as const;

/**
 * 采集来源 — one row per event source the collector can write, with the exact
 * shape of what lands on disk (verified against `collector.rs`) and its counters.
 *
 * All five sources appear, including `companion_dialogues`, which defaults ON and
 * had no switch anywhere before: a collection source the user can neither see nor
 * disable is the problem this page exists to solve.
 */
const CollectionSourcesSection: React.FC<{ settings: CollectSettingsHandle }> = ({ settings }) => {
  const { t } = useTranslation();
  const { collect, stats, patch } = settings;
  if (!collect) return null;

  return (
    <NomiSettingSection
      title={t('settings.privacy.sources.title', { defaultValue: '采集来源' })}
      description={t('settings.privacy.sources.desc', {
        defaultValue: '关掉一项后不再新增这类记录；已经记下的仍会参与学习，直到被下方的保留策略清理。',
      })}
    >
      <NomiSettingList>
        {COLLECTION_SOURCE_KEYS.map((key) => {
          const sensitivity = SOURCE_SENSITIVITY[key];
          const stat = stats.find((entry) => entry.source === key);
          return (
            <NomiSettingRow
              key={key}
              title={
                <span className='flex items-center gap-8px'>
                  <span className='text-14px font-500 text-t-primary'>
                    {t(`settings.privacy.sources.items.${key}.name`)}
                  </span>
                  <Tag size='small' color={SENSITIVITY_COLOR[sensitivity]}>
                    {t(`settings.privacy.sources.sensitivity.${sensitivity}`)}
                  </Tag>
                </span>
              }
              description={t(`settings.privacy.sources.items.${key}.desc`)}
              controls={
                <>
                  <div className='shrink-0 text-right text-12px leading-18px text-t-secondary'>
                    <div>
                      {t('settings.privacy.sources.today', { defaultValue: '今日' })}: {stat?.today ?? 0}
                    </div>
                    <div>
                      {t('settings.privacy.sources.total', { defaultValue: '当前保留' })}: {stat?.total ?? 0}
                    </div>
                  </div>
                  <Switch
                    size='small'
                    className='compact-dark-switch shrink-0'
                    checked={collect[key]}
                    onChange={(checked) => {
                      void patch({ [key]: checked }).catch((e) => Message.error(String(e)));
                    }}
                  />
                </>
              }
            />
          );
        })}
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default CollectionSourcesSection;
