/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Message, Popconfirm } from '@arco-design/web-react';
import { Attention } from '@icon-park/react';
import NomiInputNumber from '@/renderer/components/base/NomiInputNumber';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import {
  CAPACITY_MB_MAX,
  CAPACITY_MB_MIN,
  RETENTION_DAYS_MAX,
  RETENTION_DAYS_MIN,
  formatBytes,
  type CollectSettingsHandle,
} from './useCollectSettings';

/**
 * 保留策略 — the two numbers that decide how long raw events survive, plus what
 * is on disk right now.
 *
 * The copy states the real semantics, verified in
 * `crates/backend/nomifun-companion/src/collector.rs::prune_event_store`:
 *   - Day-files are bucketed by LOCAL date; the cutoff is
 *     `today - (retention_days - 1)`, so the window includes today.
 *   - An expired day-file is deleted only once every *enabled* consumer's cursor
 *     (`active_consumer_watermark` = min over learn/evolve, and only for the ones
 *     switched on) has passed all of its events. With both consumers off the
 *     watermark is `None` and expired files go immediately.
 *   - The byte cap then runs unconditionally and deletes oldest-first *ignoring
 *     the cursors*, so it can drop days nothing has read. It is enforced both by
 *     the 6-hourly prune and by every single append, so the cap always wins.
 *   - `patch_config` prunes immediately when either number changes, which is why
 *     lowering one asks for confirmation.
 */
const RetentionSection: React.FC<{ settings: CollectSettingsHandle }> = ({ settings }) => {
  const { t } = useTranslation();
  const { collect, storage, storageState, patch, refreshMeasurements } = settings;
  const [retentionDraft, setRetentionDraft] = useState<number | null>(null);
  const [capacityDraft, setCapacityDraft] = useState<number | null>(null);
  const [applying, setApplying] = useState(false);
  const savedRetention = collect?.event_retention_days ?? null;
  const savedCapacity = collect?.event_max_storage_mb ?? null;

  useEffect(() => {
    if (savedRetention != null) setRetentionDraft(savedRetention);
    if (savedCapacity != null) setCapacityDraft(savedCapacity);
  }, [savedRetention, savedCapacity]);

  if (!collect) return null;

  const retentionValid =
    retentionDraft != null &&
    Number.isInteger(retentionDraft) &&
    retentionDraft >= RETENTION_DAYS_MIN &&
    retentionDraft <= RETENTION_DAYS_MAX;
  const capacityValid =
    capacityDraft != null &&
    Number.isInteger(capacityDraft) &&
    capacityDraft >= CAPACITY_MB_MIN &&
    capacityDraft <= CAPACITY_MB_MAX;
  const changed =
    retentionValid &&
    capacityValid &&
    (retentionDraft !== collect.event_retention_days || capacityDraft !== collect.event_max_storage_mb);
  // Either number going DOWN can delete files on the spot (the PATCH prunes).
  const lowers =
    retentionValid &&
    capacityValid &&
    (retentionDraft < collect.event_retention_days || capacityDraft < collect.event_max_storage_mb);

  const apply = async () => {
    if (!retentionValid || !capacityValid) return;
    setApplying(true);
    try {
      await patch({ event_retention_days: retentionDraft, event_max_storage_mb: capacityDraft });
      refreshMeasurements(true);
      Message.success(t('settings.privacy.retention.applied', { defaultValue: '保留策略已应用' }));
    } catch (error) {
      Message.error(String(error));
    } finally {
      setApplying(false);
    }
  };

  const applyButton = (
    <Button
      type='primary'
      size='small'
      disabled={!changed}
      loading={applying}
      onClick={lowers ? undefined : () => void apply()}
    >
      {t('settings.privacy.retention.apply', { defaultValue: '应用保留策略' })}
    </Button>
  );

  const usage = (() => {
    if (storageState === 'error') {
      return (
        <span className='text-t-tertiary'>
          {t('settings.privacy.retention.unavailable', { defaultValue: '暂时无法读取存储状态。' })}
        </span>
      );
    }
    if (!storage) {
      return (
        <span className='text-t-tertiary'>
          {t('settings.privacy.retention.loading', { defaultValue: '正在读取存储状态…' })}
        </span>
      );
    }
    return (
      <span className='flex flex-wrap items-center gap-x-16px gap-y-2px'>
        <span>
          {t('settings.privacy.retention.usage', {
            used: formatBytes(storage.total_bytes),
            max: formatBytes(storage.max_bytes),
            defaultValue: '当前占用：{{used}} / {{max}}',
          })}
        </span>
        <span>
          {storage.oldest_day && storage.newest_day
            ? t('settings.privacy.retention.range', {
                from: storage.oldest_day,
                to: storage.newest_day,
                count: storage.file_count,
                defaultValue: '数据范围：{{from}} 至 {{to}}（{{count}} 个日文件）',
              })
            : t('settings.privacy.retention.empty', { defaultValue: '当前没有采集事件文件。' })}
        </span>
      </span>
    );
  })();

  return (
    <NomiSettingSection
      title={t('settings.privacy.retention.title', { defaultValue: '保留策略' })}
      description={t('settings.privacy.retention.desc', {
        defaultValue:
          '保留期按本地日期分文件计算，含今天在内。过期的日文件不会立刻删除：只有当所有已开启的学习任务（定时学习、技能生成）都读过那一天的记录后才会删；两者都关着时，过期即删。',
      })}
      action={
        lowers ? (
          <Popconfirm
            title={t('settings.privacy.retention.lowerConfirm', {
              defaultValue:
                '调低保留期或容量上限会立即执行一次清理，可能删掉最旧的原始记录，且无法恢复。已提炼的记忆和技能会保留。继续？',
            })}
            okButtonProps={{ status: 'danger' }}
            onOk={apply}
          >
            {applyButton}
          </Popconfirm>
        ) : (
          applyButton
        )
      }
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('settings.privacy.retention.days', { defaultValue: '目标保留期' })}
          description={t('settings.privacy.retention.daysDesc', {
            min: RETENTION_DAYS_MIN,
            max: RETENTION_DAYS_MAX,
            defaultValue: '原始事件最长保留多少天（{{min}}–{{max}}）。',
          })}
          controls={
            <NomiInputNumber
              contentFit
              min={RETENTION_DAYS_MIN}
              max={RETENTION_DAYS_MAX}
              precision={0}
              value={retentionDraft ?? undefined}
              onChange={(value) => {
                const parsed = Number(value);
                setRetentionDraft(Number.isFinite(parsed) ? parsed : null);
              }}
              suffix={t('settings.privacy.retention.daysUnit', { defaultValue: '天' })}
            />
          }
        />
        <NomiSettingRow
          title={t('settings.privacy.retention.capacity', { defaultValue: '容量上限' })}
          leading={
            <Attention
              theme='filled'
              size={14}
              fill='currentColor'
              // `text-danger-6` (a project uno rule) and NOT `text-danger-6`:
              // the arbitrary-value form compiles to `rgb(var(--danger-6) / var(--un-text-opacity))`,
              // which is invalid against Arco's comma-separated `--danger-6` and is
              // dropped by the CSS parser — verified in a browser, the icon just
              // inherited its parent colour.
              className='line-height-0 shrink-0 text-danger-6'
            />
          }
          description={t('settings.privacy.retention.capacityDesc', {
            min: CAPACITY_MB_MIN,
            max: CAPACITY_MB_MAX,
            defaultValue:
              '事件目录的硬上限（{{min}}–{{max}} MiB），且优先于保留期：占用一旦超过上限，就从最旧的日文件开始删，即使那天还没被学习读过。',
          })}
          controls={
            <NomiInputNumber
              contentFit
              min={CAPACITY_MB_MIN}
              max={CAPACITY_MB_MAX}
              precision={0}
              value={capacityDraft ?? undefined}
              onChange={(value) => {
                const parsed = Number(value);
                setCapacityDraft(Number.isFinite(parsed) ? parsed : null);
              }}
              suffix='MiB'
            />
          }
        />
        <NomiSettingRow
          title={t('settings.privacy.retention.usageTitle', { defaultValue: '当前占用' })}
          description={usage}
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default RetentionSection;
