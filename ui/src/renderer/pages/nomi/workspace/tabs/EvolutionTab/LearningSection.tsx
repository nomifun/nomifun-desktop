/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { Message, Switch } from '@arco-design/web-react';
import { Right } from '@icon-park/react';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import InstallWideNote from './InstallWideNote';
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
 * Pointer to 设置 › 数据采集, which owns every `collect.*` field.
 *
 * This row used to be three switches writing `collect.tool_calls` /
 * `chat_user_messages` / `requirements` — the same global fields the settings page
 * edits. Two surfaces writing one global value is worse than one, so this tab only
 * links now. It is deliberately NOT a per-companion source selector: no such field
 * exists on the profile yet.
 */
const CollectionLink: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const open = () => {
    void navigate('/settings/privacy');
  };
  return (
    <div
      role='button'
      tabIndex={0}
      onClick={open}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          open();
        }
      }}
      className='flex cursor-pointer items-center gap-4px text-12px leading-18px text-primary-6 hover:opacity-80'
    >
      <span>{t('nomi.evolution.openCollectionSettings', { defaultValue: '前往数据采集设置' })}</span>
      <Right theme='outline' size='12' fill='currentColor' strokeWidth={3} className='line-height-0 shrink-0' />
    </div>
  );
};

/**
 * 学习配置 — whether this companion reviews your work records on a schedule,
 * how often, and with which model.
 */
const LearningSection: React.FC<Props> = ({ config, needsModel }) => {
  const { t } = useTranslation();
  const { learn, patchLearn } = config;
  if (!learn) return null;

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
          title={t('nomi.evolution.collectionScopeTitle', { defaultValue: '学习素材来自哪里' })}
          description={t('nomi.evolution.collectionScopeDesc', {
            defaultValue:
              '伙伴只能学到这台设备记录下来的工作数据。记什么、留多久是应用级设置，对所有伙伴共同生效。',
          })}
          controls={<CollectionLink />}
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default LearningSection;
