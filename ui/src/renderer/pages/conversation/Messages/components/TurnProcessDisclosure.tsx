/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TurnDisclosureProcessState } from '../turnDisclosureModel';
import { Right } from '@icon-park/react';
import classNames from 'classnames';
import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
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

interface TurnProcessDisclosureCommonProps<T> {
  highlighted?: boolean;
  renderProcessItem: (item: T) => React.ReactNode;
  getProcessItemKey: (item: T) => string;
  getProcessItemState: (item: T) => TurnDisclosureProcessState;
  getProcessItemLayoutKind?: (item: T) => string;
  getProcessGroupId?: (item: T) => string | undefined;
  getProcessGroupLabel?: (items: T[]) => string;
  isProcessItemHighlighted?: (item: T) => boolean;
  getProcessItemSourceId?: (item: T) => string | undefined;
  processFooter?: React.ReactNode;
  activityLabel?: string;
}

type TurnProcessDisclosureProps<T> = TurnProcessDisclosureCommonProps<T> & (
  | { item: TurnProcessDisclosureView<T>; preparationLabel?: never }
  /** A live submit has no Turn timing until the normal timeline supplies it. */
  | { item?: undefined; preparationLabel: string }
);

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
  startAt?: number;
  endAt?: number;
  running: boolean;
  state: TurnDisclosureProcessState;
  preparationLabel?: string;
}> = ({ startAt, endAt, running, state, preparationLabel }) => {
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
  const durationMs = typeof durationEndAt === 'number' && typeof startAt === 'number' ? durationEndAt - startAt : NaN;
  const durationLabel = Number.isFinite(durationMs) && durationMs >= 0
    ? t('messages.turnDuration', {
        duration: formatTurnDuration(durationMs, t),
        defaultValue: 'Took {{duration}}',
      })
    : t('messages.turnDurationUnknown', { defaultValue: 'Time --' });
  const label = state === 'canceled'
    ? `${t('messages.canceledExecution', { defaultValue: 'Execution canceled' })} · ${durationLabel}`
    : durationLabel;
  return <span className='turn-process-disclosure__label' role={preparationLabel ? 'status' : undefined}>{preparationLabel ?? label}</span>;
};

type ProcessEntry<T> = { kind: 'item'; item: T } | { kind: 'group'; id: string; items: T[] };

/** A group is a read-only canonical boundary. Manual choices are scoped to its
 * immutable identity and survive incoming text, tools, and history refreshes. */
function ProcessGroup<T>({ id, items, label, active, running, highlighted, children }: {
  id: string; items: T[]; label: string; active: boolean; running: boolean; highlighted: boolean; children: React.ReactNode;
}) {
  const [choice, setChoice] = useState<boolean | undefined>();
  const expanded = choice ?? active;
  const bodyRef = useRef<HTMLDivElement>(null);
  const followRef = useRef(true);
  useLayoutEffect(() => {
    if (active && expanded && followRef.current && bodyRef.current) bodyRef.current.scrollTop = bodyRef.current.scrollHeight;
  }, [active, expanded, items]);
  const bodyId = `process-group-${sanitizeDomId(id)}`;
  useEffect(() => { if (highlighted) setChoice(true); }, [highlighted]);
  return <div className={classNames('turn-process-group', running && 'turn-process-group--running')} data-process-group-id={id}>
    <button type='button' className='turn-process-group__toggle' aria-expanded={expanded} aria-controls={bodyId}
      onClick={() => setChoice(!expanded)}>
      <Right theme='outline' size='12' className={classNames('turn-process-disclosure__arrow', expanded && 'turn-process-disclosure__arrow--open')} />
      <span className='turn-process-group__label'>{label}</span>
    </button>
    {expanded && <div ref={bodyRef} id={bodyId} className='turn-process-group__body' data-process-group-items={items.length}
      onWheel={event => { if (event.deltaY < 0) followRef.current = false; }}
      onScroll={event => { const body = event.currentTarget; followRef.current = body.scrollHeight - body.scrollTop - body.clientHeight < 16; }}>
      {children}
    </div>}
  </div>;
}

function TurnProcessDisclosure<T>({
  item,
  preparationLabel,
  highlighted = false,
  renderProcessItem,
  getProcessItemKey,
  getProcessItemState,
  getProcessItemLayoutKind,
  getProcessGroupId,
  getProcessGroupLabel,
  isProcessItemHighlighted,
  getProcessItemSourceId,
  processFooter,
  activityLabel,
}: TurnProcessDisclosureProps<T>) {
  const { t } = useTranslation();
  const hasProcessItems = Boolean(item && (item.processItems.length > 0 || processFooter != null));
  const itemId = item?.id ?? '';
  const defaultCollapsed = item?.defaultCollapsed ?? true;
  const running = item?.running === true;
  const beganPreparing = useRef(!item);
  const [expanded, setExpanded] = useState(() => getDefaultExpanded(hasProcessItems, defaultCollapsed));
  const expansionSnapshotRef = useRef<TurnProcessDisclosureExpansionSnapshot>({
    itemId,
    hasProcessItems,
    defaultCollapsed,
    running,
  });

  useEffect(() => {
    const nextSnapshot: TurnProcessDisclosureExpansionSnapshot = {
      itemId,
      hasProcessItems,
      defaultCollapsed,
      running,
    };
    const shouldReset = shouldResetTurnProcessDisclosureExpansion(expansionSnapshotRef.current, nextSnapshot);
    expansionSnapshotRef.current = nextSnapshot;
    if (shouldReset) setExpanded(getDefaultExpanded(hasProcessItems, defaultCollapsed));
  }, [hasProcessItems, defaultCollapsed, itemId]);

  useEffect(() => {
    if (highlighted && hasProcessItems) setExpanded(true);
  }, [hasProcessItems, highlighted]);

  const entries = useMemo(() => {
    const output: ProcessEntry<T>[] = [];
    const groups = new Map<string, Extract<ProcessEntry<T>, { kind: 'group' }>>();
    for (const processItem of item?.processItems ?? []) {
      const groupId = getProcessGroupId?.(processItem);
      if (!groupId) { output.push({ kind: 'item', item: processItem }); continue; }
      let group = groups.get(groupId);
      if (!group) { group = { kind: 'group', id: groupId, items: [] }; groups.set(groupId, group); output.push(group); }
      group.items.push(processItem);
    }
    return output;
  }, [getProcessGroupId, item?.processItems]);
  const currentGroup = item?.running ? entries.findLast(entry => entry.kind === 'group' &&
    entry.items.some(processItem => getProcessItemState(processItem) === 'running')) : undefined;
  const currentGroupId = currentGroup?.kind === 'group' ? currentGroup.id : undefined;
  // A trailing process phase also includes the model/argument wait between its
  // tool calls. Narration closes that continuation; lifecycle status rows do
  // not. This affects the title only, never a tool outcome or expansion choice.
  const continuingPhase = running ? entries.findLast(entry => entry.kind === 'group'
    || getProcessItemLayoutKind?.(entry.item) === 'text') : undefined;
  const continuingGroupId = continuingPhase?.kind === 'group' ? continuingPhase.id : undefined;
  const renderEntryItem = (processItem: T) => {
    const itemKey = getProcessItemKey(processItem);
    const state = getProcessItemState(processItem);
    const layoutKind = getProcessItemLayoutKind?.(processItem) ?? 'other';
    return <div key={itemKey} data-process-segment-id={itemKey} data-process-source-message-id={getProcessItemSourceId?.(processItem)}
      className={classNames('turn-process-disclosure__item', `turn-process-disclosure__item--${layoutKind}`,
        `turn-process-disclosure__item--${state}`)}>
      {renderProcessItem(processItem)}
    </div>;
  };

  const bodyId = `turn-process-disclosure-body-${sanitizeDomId(itemId)}`;
  const disclosureExpanded = hasProcessItems && (beganPreparing.current && expansionSnapshotRef.current.itemId === ''
    ? getDefaultExpanded(hasProcessItems, defaultCollapsed) : expanded);
  const headerContent = (
    <>
      <TurnWorkDuration startAt={item?.startAt} endAt={item?.endAt} running={running} state={item?.state ?? 'completed'} preparationLabel={!item ? preparationLabel : undefined} />
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
        `turn-process-disclosure--${item?.state ?? 'preparing'}`,
        running && 'turn-process-disclosure--live'
      )}
    >
      <div className={classNames('turn-process-disclosure__header', !hasProcessItems && 'turn-process-disclosure__header--static')}>
        {hasProcessItems || beganPreparing.current ? (
          <button
            type='button'
            className={classNames('turn-process-disclosure__toggle', !hasProcessItems && 'turn-process-disclosure__toggle--static')}
            disabled={!hasProcessItems}
            onClick={() => setExpanded((value) => !value)}
            aria-label={hasProcessItems ? t(
              disclosureExpanded ? 'messages.turnProcess.collapse' : 'messages.turnProcess.expand',
              { defaultValue: disclosureExpanded ? 'Collapse thinking process' : 'Expand thinking process' }
            ) : undefined}
            aria-expanded={hasProcessItems ? disclosureExpanded : undefined}
            aria-controls={hasProcessItems ? bodyId : undefined}
          >
            {headerContent}
          </button>
        ) : (
          <div className='turn-process-disclosure__toggle turn-process-disclosure__toggle--static'>
            {headerContent}
          </div>
        )}
      </div>
      {item?.state === 'canceled' && !item.running && item.hasInterruptedReply && (
        <div data-testid='interrupted-reply-notice' className='mt-4px text-12px text-t-secondary'>
          {t('messages.turnProcess.interruptedReply', {
            defaultValue: 'The reply below was written before stopping and is incomplete.',
          })}
        </div>
      )}
      {disclosureExpanded && item && (
        <div id={bodyId} className='turn-process-disclosure__body'>
          {entries.map(entry => entry.kind === 'item' ? renderEntryItem(entry.item) :
            <ProcessGroup key={entry.id} id={entry.id} items={entry.items}
              label={getProcessGroupLabel?.(entry.items) ?? t('conversation.thinking.complete')}
              running={running && (entry.id === continuingGroupId
                || entry.items.some(processItem => getProcessItemState(processItem) === 'running'))}
              active={entry.id === currentGroupId} highlighted={entry.items.some(processItem => isProcessItemHighlighted?.(processItem))}>
              {entry.items.map(renderEntryItem)}
            </ProcessGroup>)}
          {processFooter}
        </div>
      )}
    </div>
  );
}

export default TurnProcessDisclosure;
