/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Message, TimePicker } from '@arco-design/web-react';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import type { ICompanionProfile } from '@/common/adapter/ipcBridge';
import type { CompanionHandle } from '../../types';

interface Props {
  profile: ICompanionProfile;
  patchCompanion: CompanionHandle['patchCompanion'];
}

/**
 * 休眠时段 — per companion, stored on the profile's appearance. An empty range
 * means the companion may speak up around the clock.
 *
 * The window now gates this companion's 定时学习 and 技能进化 ticks too, not just
 * its bubbles: a background LLM run spends the owner's money and writes memories,
 * which is not what 休眠 should mean. IM auto-replies are deliberately NOT gated —
 * silently not answering a message would be a surprise.
 */
const QuietHoursSection: React.FC<Props> = ({ profile, patchCompanion }) => {
  const { t } = useTranslation();
  const { quiet_start, quiet_end } = profile.appearance;

  return (
    <NomiSettingSection
      title={t('nomi.evolution.quietTitle', { defaultValue: '休眠时段' })}
      description={t('nomi.evolution.quietSectionDesc', {
        defaultValue: '给这个伙伴安排一段安静时间：时段内它不主动找你，后台学习也一起歇着。',
      })}
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.evolution.quietRange', { defaultValue: '休眠时段' })}
          description={t('nomi.evolution.quietRangeDesc', {
            defaultValue:
              '时段内桌面伙伴不再弹气泡打扰你，未读提醒仍会累计到它的角标上；定时学习和技能生成也会跳过，等睡醒再继续。收到消息仍会正常回复。留空表示全天都可以打扰。',
          })}
          controls={
            <TimePicker.RangePicker
              format='HH:mm'
              allowClear
              className='nomi-quiet-hours-picker !h-36px !w-260px shrink-0 !bg-[var(--color-bg-1)] !border-[var(--color-border-2)] !rd-8px max-[760px]:!w-full'
              value={quiet_start && quiet_end ? [quiet_start, quiet_end] : undefined}
              onChange={(value) => {
                const [start, end] = (value as string[] | undefined) ?? ['', ''];
                void patchCompanion({
                  appearance: { quiet_start: start || '', quiet_end: end || '' },
                }).catch((e) => Message.error(String(e)));
              }}
            />
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default QuietHoursSection;
