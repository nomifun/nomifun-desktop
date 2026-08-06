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
 * nothing more: it writes `collect.* = false` for all five sources, then
 * `learn.enabled = false` + `evolve.enabled = false` on EVERY companion. It does
 * NOT delete anything — models, intervals, learned skills, distilled memories and
 * the already-collected event files all survive, the last of those still governed
 * by the retention policy above.
 *
 * Two things the copy must not hide. It is not one atomic write: collection is one
 * shared file and learning is N profiles, so the halves land separately
 * (`service.rs::disable_all` documents the chosen interleaving). And because it
 * spans the whole roster, this button reaches beyond the companion whose tab it
 * sits in — the only control here that does.
 */
const StopAllSection: React.FC<{ settings: CollectSettingsHandle }> = ({ settings }) => {
  const { t } = useTranslation();
  const [stopping, setStopping] = useState(false);

  const stopAll = async () => {
    setStopping(true);
    try {
      const outcome = await settings.disableAll();
      if (outcome.complete) {
        Message.success(t('nomi.collect.stopAll.done', { defaultValue: '已全部关闭' }));
      } else if (outcome.collectionStopped) {
        // The half that landed is the half the user cares about most, so lead with
        // it. A bare error here would read as "nothing happened" and invite a
        // second press — while recording has in fact already stopped. Retrying is
        // safe (the write is idempotent), so say so.
        Message.error(
          t('nomi.collect.stopAll.partial', {
            defaultValue: '采集已全部停止，但有伙伴的学习没能停下。再按一次即可重试，已经停下的不受影响。',
          })
        );
      } else {
        Message.error(outcome.error ?? t('nomi.collect.stopAll.failed', { defaultValue: '没能关闭，请重试。' }));
      }
    } catch (error) {
      Message.error(String(error));
    } finally {
      setStopping(false);
    }
  };

  return (
    <NomiSettingSection
      title={t('nomi.collect.stopAll.title', { defaultValue: '全部停止' })}
      // `!` because NomiSettingSection's own `text-t-primary` would otherwise win
      // on source order, and plain `text-danger-6` emits invalid CSS.
      titleClassName='!text-danger-6'
      description={t('nomi.collect.stopAll.desc', {
        defaultValue: '一次关掉上面所有采集来源，同时停掉所有伙伴的定时学习和技能生成。',
      })}
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.collect.stopAll.action', { defaultValue: '一键全关' })}
          description={t('nomi.collect.stopAll.actionDesc', {
            defaultValue:
              '只改开关，不删数据：模型和周期设置保持不变，已学到的技能和记忆保留，已采集的原始记录仍按上面的保留策略自动清理。它会停掉所有伙伴的学习，不只是当前这个。',
          })}
          controls={
            <Popconfirm
              title={t('nomi.collect.stopAll.confirm', {
                defaultValue:
                  '关闭全部采集来源，并停止所有伙伴的定时学习与技能生成？随时可以再开启，无需重新配置模型。已采集的原始记录不会被删除。',
              })}
              okButtonProps={{ status: 'danger' }}
              onOk={stopAll}
            >
              <Button status='danger' type='primary' size='small' loading={stopping} className='shrink-0'>
                {t('nomi.collect.stopAll.action', { defaultValue: '一键全关' })}
              </Button>
            </Popconfirm>
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default StopAllSection;
