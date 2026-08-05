/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Switch } from '@arco-design/web-react';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import InstallWideNote from './InstallWideNote';
import LearningModelRow from './LearningModelRow';
import NumberSetting from './NumberSetting';
import { LEARNING_SOURCE_KEYS, type EvolutionConfigHandle, type LearningSources } from './useEvolutionConfig';

const SWITCH_PROPS = { size: 'small' as const, className: 'compact-dark-switch shrink-0' };

interface Props {
  config: EvolutionConfigHandle;
  /** Learning or skill generation is on, but no learning model is selected. */
  needsModel: boolean;
}

/** The three learning-source toggles, grouped under one row (one idea: 数据范围). */
const SourceToggles: React.FC<{
  sources: LearningSources;
  patchSources: EvolutionConfigHandle['patchSources'];
}> = ({ sources, patchSources }) => {
  const { t } = useTranslation();
  return (
    <div className='flex flex-col gap-10px'>
      {LEARNING_SOURCE_KEYS.map((key) => (
        <div key={key} className='flex min-w-0 items-center gap-12px'>
          <div className='min-w-0 flex-1'>
            <div className='text-13px leading-19px font-500 text-t-primary'>
              {t(`nomi.collect.sources.${key}.name`)}
            </div>
            <div className='mt-1px text-12px leading-18px text-t-tertiary'>
              {t(`nomi.collect.sources.${key}.desc`)}
            </div>
          </div>
          <Switch
            {...SWITCH_PROPS}
            checked={sources[key]}
            onChange={(checked) => {
              void patchSources({ [key]: checked }).catch((e) => Message.error(String(e)));
            }}
          />
        </div>
      ))}
    </div>
  );
};

/**
 * 学习配置 — whether this companion reviews your work records on a schedule,
 * how often, with which model, and which recorded sources it may read.
 */
const LearningSection: React.FC<Props> = ({ config, needsModel }) => {
  const { t } = useTranslation();
  const { learn, sources, patchLearn, patchSources } = config;
  if (!learn || !sources) return null;

  return (
    <NomiSettingSection
      title={t('nomi.learn.sectionTitle', { defaultValue: '学习配置' })}
      description={
        <>
          {t('nomi.evolution.learningDesc', {
            defaultValue: '伙伴会定期回顾你的工作记录，从中提炼记忆和经验。',
          })}
          {config.installWide && (
            <InstallWideNote
              text={
                config.ownsLearningOutput
                  ? undefined
                  : t('nomi.evolution.learnNotOwnerNote', {
                      defaultValue:
                        '这组设置目前对所有伙伴共同生效；定时学习提炼出的记忆会归到默认伙伴名下，不会出现在这个伙伴的记忆页。',
                    })
              }
            />
          )}
        </>
      }
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
        <NomiSettingRow
          title={t('nomi.evolution.sourcesTitle', { defaultValue: '学习数据范围' })}
          description={t('nomi.evolution.sourcesDesc', {
            defaultValue: '选择哪些工作记录会被记下来当作学习素材。关掉一项后不再新增这类记录，已经记下的仍会参与学习。',
          })}
          footer={<SourceToggles sources={sources} patchSources={patchSources} />}
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default LearningSection;
