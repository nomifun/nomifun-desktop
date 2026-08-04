/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, InputNumber, Message, Popconfirm, Spin, Switch, Tag } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type {
  ICompanionEventStorageStatus,
  ICompanionSourceStats,
} from '@/common/adapter/ipcBridge';
import type { useCompanionShared } from '../useNomi';

type CollectionSourceKey = 'tool_calls' | 'chat_user_messages' | 'requirements' | 'terminal_sessions';

const SOURCES: { key: CollectionSourceKey; sensitivity: 'low' | 'medium' | 'high' }[] = [
  { key: 'tool_calls', sensitivity: 'medium' },
  { key: 'chat_user_messages', sensitivity: 'high' },
  { key: 'requirements', sensitivity: 'medium' },
  { key: 'terminal_sessions', sensitivity: 'medium' },
];

const SENSITIVITY_COLOR = { low: 'green', medium: 'orange', high: 'red' } as const;

const formatBytes = (bytes: number): string => {
  if (bytes < 1024 * 1024) return `${Math.max(0, bytes / 1024).toFixed(1)} KiB`;
  return `${Math.max(0, bytes / (1024 * 1024)).toFixed(1)} MiB`;
};

interface Props {
  shared: ReturnType<typeof useCompanionShared>;
}

const CollectTab: React.FC<Props> = ({ shared }) => {
  const { t } = useTranslation();
  const { sharedConfig, patchSharedConfig } = shared;
  const [stats, setStats] = useState<ICompanionSourceStats[]>([]);
  const [storage, setStorage] = useState<ICompanionEventStorageStatus | null>(null);
  const [storageLoading, setStorageLoading] = useState(true);
  const [storageError, setStorageError] = useState(false);
  const [retentionDraft, setRetentionDraft] = useState<number | null>(null);
  const [capacityDraft, setCapacityDraft] = useState<number | null>(null);
  const [applyingPolicy, setApplyingPolicy] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const storageRequestRef = useRef(0);

  const refreshStorage = (showLoading: boolean) => {
    const storageRequest = ++storageRequestRef.current;
    if (showLoading) {
      setStorage(null);
      setStorageLoading(true);
      setStorageError(false);
    }
    void ipcBridge.companion.eventStorage
      .invoke()
      .then((nextStorage) => {
        if (storageRequest !== storageRequestRef.current) return;
        setStorage(nextStorage);
        setStorageError(false);
      })
      .catch(() => {
        if (storageRequest !== storageRequestRef.current) return;
        setStorage(null);
        setStorageError(true);
      })
      .finally(() => {
        if (storageRequest === storageRequestRef.current) setStorageLoading(false);
      });
  };

  const refreshStats = () => {
    void ipcBridge.companion.eventStats
      .invoke()
      .then(setStats)
      .catch(() => {});
    refreshStorage(false);
  };

  useEffect(() => {
    refreshStats();
    // Counters move as events stream in; poll lightly while the tab is open
    // and refresh on learn completion (which consumes events). Arco keeps
    // inactive panes mounted (display:none), so skip polls while hidden —
    // offsetParent is null for display:none subtrees.
    const timer = setInterval(() => {
      if (rootRef.current?.offsetParent != null) refreshStats();
    }, 15_000);
    const unsubLearn = ipcBridge.companion.onLearnFinished.on(refreshStats);
    return () => {
      clearInterval(timer);
      unsubLearn();
      storageRequestRef.current += 1;
    };
  }, []);

  useEffect(() => {
    if (!sharedConfig) return;
    setRetentionDraft(sharedConfig.collect.event_retention_days);
    setCapacityDraft(sharedConfig.collect.event_max_storage_mb);
  }, [sharedConfig?.collect.event_retention_days, sharedConfig?.collect.event_max_storage_mb]);

  if (!sharedConfig) {
    return (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    );
  }

  const statFor = (key: string) => stats.find((s) => s.source === key);
  const retentionValid =
    retentionDraft != null && Number.isInteger(retentionDraft) && retentionDraft >= 7 && retentionDraft <= 365;
  const capacityValid =
    capacityDraft != null && Number.isInteger(capacityDraft) && capacityDraft >= 16 && capacityDraft <= 512;
  const policyChanged =
    retentionValid &&
    capacityValid &&
    (retentionDraft !== sharedConfig.collect.event_retention_days ||
      capacityDraft !== sharedConfig.collect.event_max_storage_mb);
  const lowersPolicy =
    retentionValid &&
    capacityValid &&
    (retentionDraft < sharedConfig.collect.event_retention_days ||
      capacityDraft < sharedConfig.collect.event_max_storage_mb);

  const applyStoragePolicy = async () => {
    if (!retentionValid || !capacityValid) return;
    setApplyingPolicy(true);
    try {
      await patchSharedConfig({
        collect: {
          event_retention_days: retentionDraft,
          event_max_storage_mb: capacityDraft,
        },
      });
      void ipcBridge.companion.eventStats
        .invoke()
        .then(setStats)
        .catch(() => {});
      refreshStorage(true);
      Message.success(t('nomi.collect.policyApplied'));
    } catch (error) {
      Message.error(String(error));
    } finally {
      setApplyingPolicy(false);
    }
  };

  const applyPolicyButton = (
    <Button
      type='primary'
      size='small'
      disabled={!policyChanged}
      loading={applyingPolicy}
      onClick={lowersPolicy ? undefined : () => void applyStoragePolicy()}
    >
      {t('nomi.collect.applyPolicy')}
    </Button>
  );

  return (
    <div ref={rootRef} className='flex flex-col gap-12px py-8px'>
      <p className='m-0 text-13px text-t-secondary'>{t('nomi.collect.intro')}</p>
      <div className='flex flex-col gap-8px'>
        {SOURCES.map(({ key, sensitivity }) => {
          const stat = statFor(key);
          return (
            <div key={key} className='flex items-center gap-12px bg-fill-2 rd-10px px-12px py-10px'>
              <Switch
                size='small'
                className='compact-dark-switch'
                checked={sharedConfig.collect[key]}
                onChange={(checked) => {
                  void patchSharedConfig({ collect: { [key]: checked } }).catch((e) =>
                    Message.error(String(e))
                  );
                }}
              />
              <div className='flex-1 min-w-0'>
                <div className='flex items-center gap-8px'>
                  <span className='text-14px text-t-primary font-500'>{t(`nomi.collect.sources.${key}.name`)}</span>
                  <Tag size='small' color={SENSITIVITY_COLOR[sensitivity]}>
                    {t(`nomi.collect.sensitivity.${sensitivity}`)}
                  </Tag>
                </div>
                <div className='text-12px text-t-tertiary mt-2px'>{t(`nomi.collect.sources.${key}.desc`)}</div>
              </div>
              <div className='text-12px text-t-secondary shrink-0 text-right'>
                <div>
                  {t('nomi.collect.today')}: {stat?.today ?? 0}
                </div>
                <div>
                  {t('nomi.collect.total')}: {stat?.total ?? 0}
                </div>
              </div>
            </div>
          );
        })}
      </div>
      <div className='flex items-start gap-16px bg-fill-2 rd-10px px-14px py-12px flex-wrap'>
        <div className='w-220px shrink-0'>
          <div className='text-14px text-t-primary font-500'>{t('nomi.collect.retentionTitle')}</div>
          <div className='text-12px text-t-tertiary mt-2px'>{t('nomi.collect.retentionHint')}</div>
        </div>
        <div className='flex-1 min-w-280px flex flex-col gap-10px'>
          <div className='flex items-center gap-8px flex-wrap'>
            <span className='text-13px text-t-secondary'>{t('nomi.collect.retentionDays')}</span>
            <InputNumber
              style={{ width: 120 }}
              min={7}
              max={365}
              precision={0}
              value={retentionDraft ?? undefined}
              onChange={(value) => {
                const parsed = Number(value);
                setRetentionDraft(Number.isFinite(parsed) ? parsed : null);
              }}
              suffix={t('nomi.collect.days')}
            />
          </div>
          <div className='flex items-center gap-8px flex-wrap'>
            <span className='text-13px text-t-secondary'>{t('nomi.collect.capacityLimit')}</span>
            <InputNumber
              style={{ width: 120 }}
              min={16}
              max={512}
              precision={0}
              value={capacityDraft ?? undefined}
              onChange={(value) => {
                const parsed = Number(value);
                setCapacityDraft(Number.isFinite(parsed) ? parsed : null);
              }}
              suffix='MiB'
            />
          </div>
          <div>
            {lowersPolicy ? (
              <Popconfirm title={t('nomi.collect.lowerPolicyConfirm')} onOk={applyStoragePolicy}>
                {applyPolicyButton}
              </Popconfirm>
            ) : (
              applyPolicyButton
            )}
          </div>
        </div>
        <div className='min-w-220px text-12px text-t-secondary leading-20px'>
          {storageError && !storageLoading ? (
            <div className='text-t-tertiary'>{t('nomi.collect.storageUnavailable')}</div>
          ) : storage ? (
            <>
              <div>
                {t('nomi.collect.storageUsage', {
                  used: formatBytes(storage.total_bytes),
                  max: formatBytes(storage.max_bytes),
                })}
              </div>
              <div>
                {storage.oldest_day && storage.newest_day
                  ? t('nomi.collect.storedRange', {
                      from: storage.oldest_day,
                      to: storage.newest_day,
                      count: storage.file_count,
                    })
                  : t('nomi.collect.noStoredEvents')}
              </div>
              <div className='text-t-tertiary'>{t('nomi.collect.hardLimitHint')}</div>
            </>
          ) : (
            <div className='text-t-tertiary'>{t('nomi.collect.storageLoading')}</div>
          )}
        </div>
      </div>
      <div className='flex items-center gap-12px flex-wrap'>
        <Popconfirm
          title={t('nomi.collect.disableAllConfirm', {
            defaultValue: '停止所有采集、学习与进化？已学到的技能和记忆会保留，模型配置不变，随时可再开启。',
          })}
          onOk={() => {
            void ipcBridge.companion.disableAll
              .invoke()
              .then(() => {
                void shared.refresh();
                refreshStats();
                Message.success(t('nomi.collect.disabledAll', { defaultValue: '已全部关闭' }));
              })
              .catch((e) => Message.error(String(e)));
          }}
        >
          <Button status='danger' type='primary'>
            {t('nomi.collect.disableAll', { defaultValue: '一键全关' })}
          </Button>
        </Popconfirm>
      </div>
    </div>
  );
};

export default CollectTab;
