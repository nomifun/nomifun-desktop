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
}

interface TurnProcessDisclosureProps<T> {
  item: TurnProcessDisclosureView<T>;
  highlighted?: boolean;
  renderProcessItem: (item: T) => React.ReactNode;
  getProcessItemKey: (item: T) => string;
  getProcessItemState: (item: T) => TurnDisclosureProcessState;
  getProcessItemLayoutKind?: (item: T) => string;
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
  if (previous.running !== next.running) return true;
  return false;
}

const formatTurnDuration = (ms: number, t: ReturnType<typeof useTranslation>['t']): string => {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const sUnit = t('common.unit.second_short', { defaultValue: 's' });
  const mUnit = t('common.unit.minute_short', { defaultValue: 'm' });
  const hUnit = t('common.unit.hour_short', { defaultValue: 'h' });

  if (totalSeconds < 60) return `${totalSeconds}${sUnit}`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) return `${minutes}${mUnit} ${seconds}${sUnit}`;
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return `${hours}${hUnit} ${remainingMinutes}${mUnit}`;
};

function TurnProcessDisclosure<T>({
  item,
  highlighted = false,
  renderProcessItem,
  getProcessItemKey,
  getProcessItemState,
  getProcessItemLayoutKind,
}: TurnProcessDisclosureProps<T>) {
  const { t } = useTranslation();
  const hasProcessItems = item.processItems.length > 0;
  const [expanded, setExpanded] = useState(() => getDefaultExpanded(hasProcessItems, item.defaultCollapsed));
  const [now, setNow] = useState(() => Date.now());
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
  }, [hasProcessItems, item.defaultCollapsed, item.id, item.running]);

  useEffect(() => {
    if (highlighted && hasProcessItems) setExpanded(true);
  }, [hasProcessItems, highlighted]);

  useEffect(() => {
    if (!item.running) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [item.running]);

  const currentItemKey = useMemo(() => {
    if (!item.running) return undefined;
    const latestItem = item.processItems.at(-1);
    if (!latestItem || getProcessItemState(latestItem) !== 'running') return undefined;
    const latestKind = getProcessItemLayoutKind?.(latestItem);
    return latestKind === 'thinking' || latestKind === 'tool'
      ? getProcessItemKey(latestItem)
      : undefined;
  }, [getProcessItemKey, getProcessItemLayoutKind, getProcessItemState, item.processItems, item.running]);

  const durationEndAt = item.running ? now : item.endAt;
  const durationMs = durationEndAt - item.startAt;
  const durationLabel = Number.isFinite(durationMs) && durationMs >= 0
    ? t('messages.turnDuration', {
        duration: formatTurnDuration(durationMs, t),
        defaultValue: 'Took {{duration}}',
      })
    : t('messages.turnDurationUnknown', { defaultValue: 'Time --' });
  const label = item.running
    ? t('messages.turnProcess.runningSummary', {
        duration: durationLabel,
        defaultValue: 'Processing · {{duration}}',
      })
    : durationLabel;
  const bodyId = `turn-process-disclosure-body-${sanitizeDomId(item.id)}`;
  const disclosureExpanded = hasProcessItems && expanded;
  const headerContent = (
    <>
      <span className='turn-process-disclosure__label'>{label}</span>
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
        </div>
      )}
    </div>
  );
}

export default TurnProcessDisclosure;
