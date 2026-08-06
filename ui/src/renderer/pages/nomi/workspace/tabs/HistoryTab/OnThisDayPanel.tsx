/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Spin } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { ICompanionDayDigest } from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';
import DigestCard from './DigestCard';
import ReaderPanel from './ReaderPanel';
import { formatDayKey, formatMmdd, todayMmdd } from './historyFormat';

interface OnThisDayPanelProps {
  companionId: CompanionId;
}

/**
 * 「去年今日」 — the only view that queries by day, because the digest endpoint is
 * the only one that supports it (`on_day=MMDD`). Digests exist only when 会话归档
 * was enabled, so an empty result here is normal, not an error.
 */
const OnThisDayPanel: React.FC<OnThisDayPanelProps> = ({ companionId }) => {
  const { t } = useTranslation();
  const mmdd = useMemo(() => todayMmdd(), []);
  const [digests, setDigests] = useState<ICompanionDayDigest[]>([]);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setFailed(false);
    void ipcBridge.companion.listDayDigests
      .invoke({ companion_id: companionId, on_day: mmdd, limit: 60 })
      .then((rows) => {
        if (alive) setDigests(rows);
      })
      .catch(() => {
        // A failed read is not the same as "nothing happened that day".
        if (!alive) return;
        setDigests([]);
        setFailed(true);
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [companionId, mmdd]);

  const groups = useMemo(() => {
    const byDay = new Map<string, ICompanionDayDigest[]>();
    for (const digest of digests) {
      const list = byDay.get(digest.session_day) ?? [];
      list.push(digest);
      byDay.set(digest.session_day, list);
    }
    return Array.from(byDay.entries()).sort((a, b) => b[0].localeCompare(a[0]));
  }, [digests]);

  return (
    <ReaderPanel
      header={
        <>
          <span className='text-15px font-600 leading-22px text-t-primary'>
            {t('nomi.archive.onThisDay', { defaultValue: '去年今日' })}
          </span>
          <span className='ml-auto shrink-0 text-11px text-t-tertiary'>
            {t('nomi.history.onThisDayHint', { defaultValue: '往年同一天（{{day}}）的会话日记', day: formatMmdd(mmdd) })}
          </span>
        </>
      }
    >
      {loading ? (
        <div className='flex justify-center py-40px'>
          <Spin />
        </div>
      ) : groups.length === 0 ? (
        <div className='flex flex-col items-center gap-6px py-40px text-center'>
          <span className='text-13px text-t-tertiary'>
            {failed
              ? t('nomi.history.onThisDayFailed', { defaultValue: '没读到往年的日记，可能是后端暂时不可用。' })
              : t('nomi.archive.onThisDayEmpty', { defaultValue: '往年的今天还没有记录～' })}
          </span>
          <span className='max-w-320px text-11px leading-16px text-t-tertiary'>
            {failed
              ? t('nomi.history.onThisDayFailedHint', { defaultValue: '切走再回来会重新读取一次。' })
              : t('nomi.history.onThisDayArchiveNote', {
                  defaultValue: '日记来自「会话归档」（全局设置，对所有伙伴生效，默认关闭）；没开启时这里始终为空。',
                })}
          </span>
        </div>
      ) : (
        groups.map(([day, items]) => (
          <div key={day} className='flex flex-col gap-8px'>
            <div className='text-12px font-500 text-t-secondary'>{formatDayKey(day)}</div>
            {items.map((digest) => (
              <DigestCard key={digest.session_window_id} digest={digest} />
            ))}
          </div>
        ))
      )}
    </ReaderPanel>
  );
};

export default OnThisDayPanel;
