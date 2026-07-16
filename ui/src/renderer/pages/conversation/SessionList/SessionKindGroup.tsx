/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Checkbox, Tooltip } from '@arco-design/web-react';
import { Plus, Right } from '@icon-park/react';
import classNames from 'classnames';
import React from 'react';
import { useTranslation } from 'react-i18next';

import type { SessionEntry, SessionKind } from './utils/workpathTree';

export interface SessionKindGroupProps {
  kind: SessionKind;
  entries: SessionEntry[];
  totalCount?: number;
  expanded: boolean;
  onToggle: () => void;
  onCreate: () => void;
  batchMode?: boolean;
  selectionChecked?: boolean;
  selectionIndeterminate?: boolean;
  selectionDisabled?: boolean;
  onToggleSelection?: () => void;
  hasOverflow?: boolean;
  hiddenCount?: number;
  showAll?: boolean;
  onToggleShowAll?: () => void;
  /** Row renderer supplied by the container (ConversationRow / TerminalRow with all action props wired). */
  renderEntry: (entry: SessionEntry) => React.ReactNode;
}

/**
 * Second-level kind subgroup inside a workpath drawer: a small collapsible
 * header (`▼ 交互会话 (N)` / `▼ 终端会话 (N)`) with a hover "+" create button,
 * followed by the session rows. The parent skips rendering this component when
 * `entries` is empty.
 */
const SessionKindGroup: React.FC<SessionKindGroupProps> = ({
  kind,
  entries,
  totalCount = entries.length,
  expanded,
  onToggle,
  onCreate,
  batchMode = false,
  selectionChecked = false,
  selectionIndeterminate = false,
  selectionDisabled = false,
  onToggleSelection,
  hasOverflow = false,
  hiddenCount = 0,
  showAll = false,
  onToggleShowAll,
  renderEntry,
}) => {
  const { t } = useTranslation();
  const label = kind === 'interactive' ? t('sessionList.interactiveGroup') : t('sessionList.terminalGroup');
  const createLabel = kind === 'interactive' ? t('sessionList.newInteractive') : t('sessionList.newTerminal');

  return (
    <div className='min-w-0'>
      <div
        className='group/kind flex items-center gap-4px h-26px pl-22px pr-8px cursor-pointer select-none rd-6px hover:bg-fill-2 transition-colors min-w-0'
        onClick={() => {
          if (batchMode && !selectionDisabled) {
            onToggleSelection?.();
            return;
          }
          onToggle();
        }}
      >
        <button
          type='button'
          aria-label={expanded ? t('common.collapse') : t('common.expand')}
          className='size-18px flex items-center justify-center text-t-tertiary group-hover/kind:text-t-primary transition-colors shrink-0 bg-transparent border-none p-0 cursor-pointer'
          onClick={(e) => {
            e.stopPropagation();
            onToggle();
          }}
        >
          <Right
            theme='outline'
            size={12}
            className={classNames('transition-transform duration-150', { 'rotate-90': expanded })}
          />
        </button>
        {batchMode && (
          <span
            className='shrink-0 flex-center'
            onClick={(e) => {
              e.stopPropagation();
            }}
          >
            <Checkbox
              checked={selectionChecked}
              indeterminate={selectionIndeterminate}
              disabled={selectionDisabled}
              className='session-batch-selection-checkbox'
              onChange={() => onToggleSelection?.()}
            />
          </span>
        )}
        <span className='text-12px text-t-tertiary group-hover/kind:text-t-primary transition-colors font-[500] leading-none min-w-0 truncate'>
          {label}
        </span>
        <span className='text-12px text-t-tertiary leading-none shrink-0'>({totalCount})</span>
        {!batchMode && (
          <span className='ml-auto shrink-0 flex items-center' onClick={(e) => e.stopPropagation()}>
            <Tooltip content={createLabel} position='top'>
              <span
                role='button'
                tabIndex={0}
                aria-label={createLabel}
                className='hidden group-hover/kind:flex flex-center cursor-pointer transition-colors text-t-secondary hover:text-t-primary size-18px rd-4px sider-action-btn'
                onClick={(e) => {
                  e.stopPropagation();
                  onCreate();
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    e.stopPropagation();
                    onCreate();
                  }
                }}
              >
                <Plus theme='outline' size='14' fill='currentColor' className='block leading-none' />
              </span>
            </Tooltip>
          </span>
        )}
      </div>
      {expanded && (
        <div className='min-w-0 flex flex-col'>
          {entries.map((entry) => renderEntry(entry))}
          {hasOverflow && onToggleShowAll && (
            <button
              type='button'
              aria-expanded={showAll}
              className='ml-64px mt-1px mb-2px inline-flex h-20px w-fit max-w-full appearance-none items-center border-none bg-transparent p-0 text-left text-12px leading-20px text-t-secondary transition-colors cursor-pointer select-none hover:text-t-primary focus:outline-none focus-visible:text-t-primary'
              onClick={(e) => {
                e.stopPropagation();
                onToggleShowAll();
              }}
            >
              {showAll
                ? t('sessionList.collapseDisplay')
                : t('sessionList.expandDisplay', { count: hiddenCount })}
            </button>
          )}
        </div>
      )}
    </div>
  );
};

export default SessionKindGroup;
