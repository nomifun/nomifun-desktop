/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TurnDisclosureProcessState } from '../turnDisclosureModel';
import { Right } from '@icon-park/react';
import classNames from 'classnames';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

export interface TurnProcessDisclosureView<T> {
  id: string;
  processItems: T[];
  startAt: number;
  endAt: number;
  state: TurnDisclosureProcessState;
  running: boolean;
  defaultCollapsed: boolean;
  hasInterruptedReply?: boolean;
}

interface TurnProcessDisclosureProps<T> {
  item: TurnProcessDisclosureView<T>;
  highlighted?: boolean;
  renderProcessItem: (item: T) => React.ReactNode;
  getProcessItemKey: (item: T) => string;
  getProcessItemState: (item: T) => TurnDisclosureProcessState;
  getProcessItemLayoutKind?: (item: T) => string;
  processFooter?: React.ReactNode;
  activityLabel?: string;
}

export interface TurnProcessDisclosureExpansionSnapshot {
  itemId: string;
  hasProcessItems: boolean;
  defaultCollapsed: boolean;
  running: boolean;
}

const sanitizeDomId = (value: string): string => value.replace(/[^A-Za-z0-9_-]/g, '_');

const getDefaultExpanded = (hasProcessItems: boolean, defaultCollapsed: boolean): boolean =>
  hasProcessItems && !defaultCollapsed;

export function shouldResetTurnProcessDisclosureExpansion(
  previous: TurnProcessDisclosureExpansionSnapshot,
  next: TurnProcessDisclosureExpansionSnapshot
): boolean {
  if (previous.itemId !== next.itemId) return true;
  if (previous.hasProcessItems !== next.hasProcessItems) return true;
  if (previous.defaultCollapsed !== next.defaultCollapsed) return true;
  return false;
}

const formatTurnDuration = (ms: number, t: ReturnType<typeof useTranslation>['t']): string => {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const sUnit = t('messages.turnDurationUnits.second', { defaultValue: 's' });
  const mUnit = t('messages.turnDurationUnits.minute', { defaultValue: 'm' });
  const hUnit = t('messages.turnDurationUnits.hour', { defaultValue: 'h' });

  if (totalSeconds < 60) return `${totalSeconds}${sUnit}`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) return `${minutes}${mUnit} ${seconds}${sUnit}`;
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return `${hours}${hUnit} ${remainingMinutes}${mUnit} ${seconds}${sUnit}`;
};

// The clock owns its tick so a one-second update does not rerender the entire
// thinking/tool journal. Stream renders also sample wall time, and foreground
// events catch up immediately after desktop/browser timer throttling.
const TurnWorkDuration: React.FC<{
  startAt: number;
  endAt: number;
  running: boolean;
  state: TurnDisclosureProcessState;
}> = ({ startAt, endAt, running, state }) => {
  const { t } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!running) return;
    const refresh = () => setNow(Date.now());
    refresh();
    const timer = window.setInterval(refresh, 1000);
    window.addEventListener('focus', refresh);
    document.addEventListener('visibilitychange', refresh);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener('focus', refresh);
      document.removeEventListener('visibilitychange', refresh);
    };
  }, [running, startAt]);

  const durationEndAt = running ? Math.max(now, Date.now()) : endAt;
  const durationMs = durationEndAt - startAt;
  const durationLabel = Number.isFinite(durationMs) && durationMs >= 0
    ? t('messages.turnDuration', {
        duration: formatTurnDuration(durationMs, t),
        defaultValue: 'Took {{duration}}',
      })
    : t('messages.turnDurationUnknown', { defaultValue: 'Time --' });
  const label = state === 'canceled'
    ? `${t('messages.canceledExecution', { defaultValue: 'Execution canceled' })} · ${durationLabel}`
    : durationLabel;
  return <span className='turn-process-disclosure__label'>{label}</span>;
};

function TurnProcessDisclosure<T>({
  item,
  highlighted = false,
  renderProcessItem,
  getProcessItemKey,
  getProcessItemState,
  getProcessItemLayoutKind,
  processFooter,
  activityLabel,
}: TurnProcessDisclosureProps<T>) {
  const { t } = useTranslation();
  const hasProcessItems = item.processItems.length > 0 || processFooter != null;
  const [expanded, setExpanded] = useState(() => getDefaultExpanded(hasProcessItems, item.defaultCollapsed));
  const expansionSnapshotRef = useRef<TurnProcessDisclosureExpansionSnapshot>({
    itemId: item.id,
    hasProcessItems,
    defaultCollapsed: item.defaultCollapsed,
    running: item.running,
  });

  useEffect(() => {
    const nextSnapshot: TurnProcessDisclosureExpansionSnapshot = {
      itemId: item.id,
      hasProcessItems,
      defaultCollapsed: item.defaultCollapsed,
      running: item.running,
    };
    const shouldReset = shouldResetTurnProcessDisclosureExpansion(expansionSnapshotRef.current, nextSnapshot);
    expansionSnapshotRef.current = nextSnapshot;
    if (shouldReset) setExpanded(getDefaultExpanded(hasProcessItems, item.defaultCollapsed));
  }, [hasProcessItems, item.defaultCollapsed, item.id]);

  useEffect(() => {
    if (highlighted && hasProcessItems) setExpanded(true);
  }, [hasProcessItems, highlighted]);

  const currentItemKey = useMemo(() => {
    if (!item.running) return undefined;
    const latestItem = item.processItems.findLast(processItem => {
      const kind = getProcessItemLayoutKind?.(processItem);
      return getProcessItemState(processItem) === 'running' && (kind === 'thinking' || kind === 'tool');
    });
    return latestItem ? getProcessItemKey(latestItem) : undefined;
  }, [getProcessItemKey, getProcessItemLayoutKind, getProcessItemState, item.processItems, item.running]);

  const bodyId = `turn-process-disclosure-body-${sanitizeDomId(item.id)}`;
  const disclosureExpanded = hasProcessItems && expanded;
  const headerContent = (
    <>
      <TurnWorkDuration startAt={item.startAt} endAt={item.endAt} running={item.running} state={item.state} />
      {activityLabel && <span className='turn-process-disclosure__activity' role='status'>{activityLabel}</span>}
      {hasProcessItems && (
        <Right
          theme='outline'
          size='13'
          fill='currentColor'
          className={classNames(
            'turn-process-disclosure__arrow',
            disclosureExpanded && 'turn-process-disclosure__arrow--open'
          )}
        />
      )}
    </>
  );

  return (
    <div
      className={classNames(
        'turn-process-disclosure',
        `turn-process-disclosure--${item.state}`,
        item.running && 'turn-process-disclosure--live'
      )}
    >
      <div className={classNames('turn-process-disclosure__header', !hasProcessItems && 'turn-process-disclosure__header--static')}>
        {hasProcessItems ? (
          <button
            type='button'
            className='turn-process-disclosure__toggle'
            onClick={() => setExpanded((value) => !value)}
            aria-label={t(
              disclosureExpanded ? 'messages.turnProcess.collapse' : 'messages.turnProcess.expand',
              { defaultValue: disclosureExpanded ? 'Collapse thinking process' : 'Expand thinking process' }
            )}
            aria-expanded={disclosureExpanded}
            aria-controls={bodyId}
          >
            {headerContent}
          </button>
        ) : (
          <div className='turn-process-disclosure__toggle turn-process-disclosure__toggle--static'>
            {headerContent}
          </div>
        )}
      </div>
      {item.state === 'canceled' && !item.running && item.hasInterruptedReply && (
        <div data-testid='interrupted-reply-notice' className='mt-4px text-12px text-t-secondary'>
          {t('messages.turnProcess.interruptedReply', {
            defaultValue: 'The reply below was written before stopping and is incomplete.',
          })}
        </div>
      )}
      {disclosureExpanded && (
        <div id={bodyId} className='turn-process-disclosure__body'>
          {item.processItems.map((processItem) => {
            const itemKey = getProcessItemKey(processItem);
            const state = getProcessItemState(processItem);
            const layoutKind = getProcessItemLayoutKind?.(processItem) ?? 'other';
            return (
              <div
                key={itemKey}
                className={classNames(
                  'turn-process-disclosure__item',
                  `turn-process-disclosure__item--${layoutKind}`,
                  `turn-process-disclosure__item--${state}`,
                  itemKey === currentItemKey && 'turn-process-disclosure__item--current'
                )}
              >
                {renderProcessItem(processItem)}
              </div>
            );
          })}
          {processFooter}
        </div>
      )}
    </div>
  );
}

export default TurnProcessDisclosure;
