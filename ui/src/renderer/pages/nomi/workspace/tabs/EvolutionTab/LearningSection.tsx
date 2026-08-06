/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Switch } from '@arco-design/web-react';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import LearningModelRow from './LearningModelRow';
import NumberSetting from './NumberSetting';
import type { EvolutionConfigHandle } from './useEvolutionConfig';

const SWITCH_PROPS = { size: 'small' as const, className: 'compact-dark-switch shrink-0' };

interface Props {
  config: EvolutionConfigHandle;
  /** Learning or skill generation is on, but no learning model is selected. */
  needsModel: boolean;
}

/**
 * 学习配置 — whether this companion reviews your work records on a schedule,
 * how often, and with which model.
 *
 * What it may review is 采集来源, immediately below in this same tab. This section
 * used to end with a link out to 设置 › 数据采集; the two belong on one screen, so
 * there is nothing left to link to.
 */
const LearningSection: React.FC<Props> = ({ config, needsModel }) => {
  const { t } = useTranslation();
  const { learn, patchLearn } = config;
  if (!learn) return null;

  return (
    <NomiSettingSection
      title={t('nomi.learn.sectionTitle', { defaultValue: '学习配置' })}
      description={t('nomi.evolution.learningDesc', {
        defaultValue: '这个伙伴会按下面的节奏回顾你的工作记录，把提炼出的记忆记在自己名下。',
      })}
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.learn.enabled', { defaultValue: '开启定时学习' })}
          description={t('nomi.evolution.learnEnabledDesc', {
            defaultValue: '关闭后不再进行周期性学习，已提炼的记忆和技能保持不变。',
          })}
          controls={
            <Switch
              {...SWITCH_PROPS}
              checked={learn.enabled}
              onChange={(checked) => {
                void patchLearn({ enabled: checked }).catch((e) => Message.error(String(e)));
              }}
            />
          }
        />
        <NomiSettingRow
          title={t('nomi.evolution.interval', { defaultValue: '学习周期' })}
          description={t('nomi.evolution.intervalDesc', {
            defaultValue: '每隔多久回顾一次新记录。',
          })}
          controls={
            <NumberSetting
              min={5}
              max={1440}
              value={learn.interval_minutes}
              onCommit={(interval_minutes) => patchLearn({ interval_minutes })}
              suffix={t('nomi.learn.minutes', { defaultValue: '分钟' })}
            />
          }
        />
        <LearningModelRow learn={learn} patchLearn={patchLearn} missing={needsModel} />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default LearningSection;
