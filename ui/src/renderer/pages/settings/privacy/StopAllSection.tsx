/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Message, Popconfirm } from '@arco-design/web-react';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import type { CollectSettingsHandle } from './useCollectSettings';

/**
 * 全部停止 — the 一键全关 panic switch.
 *
 * The confirm copy promises exactly what `CompanionService::disable_all` does and
 * nothing more: it writes `collect.* = false` for all five sources plus
 * `learn.enabled = false` and `evolve.enabled = false` in one atomic patch. It
 * does NOT delete anything — models, intervals, learned skills, distilled
 * memories and the already-collected event files all survive, the last of those
 * still governed by the retention policy above.
 */
const StopAllSection: React.FC<{ settings: CollectSettingsHandle }> = ({ settings }) => {
  const { t } = useTranslation();
  const [stopping, setStopping] = useState(false);

  const stopAll = async () => {
    setStopping(true);
    try {
      await settings.disableAll();
      Message.success(t('settings.privacy.stopAll.done', { defaultValue: '已全部关闭' }));
    } catch (error) {
      Message.error(String(error));
    } finally {
      setStopping(false);
    }
  };

  return (
    <NomiSettingSection
      title={t('settings.privacy.stopAll.title', { defaultValue: '全部停止' })}
      // `!` because NomiSettingSection's own `text-t-primary` would otherwise win
      // on source order, and plain `text-danger-6` emits invalid CSS.
      titleClassName='!text-danger-6'
      description={t('settings.privacy.stopAll.desc', {
        defaultValue: '一次关掉上面所有采集来源，同时停掉定时学习和技能生成。',
      })}
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('settings.privacy.stopAll.action', { defaultValue: '一键全关' })}
          description={t('settings.privacy.stopAll.actionDesc', {
            defaultValue:
              '只改开关，不删数据：模型和周期设置保持不变，已学到的技能和记忆保留，已采集的原始记录仍按上面的保留策略自动清理。',
          })}
          controls={
            <Popconfirm
              title={t('settings.privacy.stopAll.confirm', {
                defaultValue:
                  '关闭全部采集来源，并停止定时学习与技能生成？随时可以再开启，无需重新配置模型。已采集的原始记录不会被删除。',
              })}
              okButtonProps={{ status: 'danger' }}
              onOk={stopAll}
            >
              <Button status='danger' type='primary' size='small' loading={stopping} className='shrink-0'>
                {t('settings.privacy.stopAll.action', { defaultValue: '一键全关' })}
              </Button>
            </Popconfirm>
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default StopAllSection;
