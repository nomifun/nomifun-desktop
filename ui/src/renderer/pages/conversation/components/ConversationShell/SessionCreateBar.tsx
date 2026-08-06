/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { ExpandLeft, ListCheckbox, Plus } from '@icon-park/react';
import classNames from 'classnames';
import InstantHoverTooltip from '@renderer/components/base/InstantHoverTooltip';
import ConversationSearchPopover from '@renderer/pages/conversation/SessionList/ConversationSearchPopover';
import type { SidebarDisplayPreferences, SidebarDisplayPreset } from '@renderer/pages/conversation/SessionList/utils/sidebarDisplayPreferences';
import RemoteSessionPopover from './RemoteSessionPopover';
import SessionDisplaySettingsPopover from './SessionDisplaySettingsPopover';

export interface SessionCreateBarProps {
  batchMode: boolean;
  onToggleBatchMode: () => void;
  onNewChat: () => void;
  onNewTerminal: () => void;
  onCreateProject: () => void;
  displayPreferences: SidebarDisplayPreferences;
  onDisplayPresetChange: (preset: Exclude<SidebarDisplayPreset, 'custom'>) => void;
  onDisplayPreferenceChange: (patch: Partial<Omit<SidebarDisplayPreferences, 'preset'>>) => void;
  /** Collapse the secondary sidebar. The stable primary toggle lives in the titlebar. */
  onCollapse: () => void;
  /** Mobile-only: close the overlay when a session is chosen from search. */
  onSessionClick?: () => void;
  /** Clear batch mode / close preview when a search result is opened. */
  onConversationSelect: () => void;
}

/**
 * SessionCreateBar — the toolbar at the top of the session secondary sidebar
 * ({@link ContentSider}). Carries the create CTAs (conversation / terminal /
 * project / remote host session), search, batch selection, display settings, and
 * an in-panel collapse shortcut.
 *
 * Two rules keep this dense without getting cryptic:
 *  - The 2x2 grid holds *only* creation actions. Each one is a `+` icon (the
 *    verb) plus a bare noun (the object), so four fit where three long
 *    "新建…" labels used to, and a fifth costs no extra reading.
 *  - Mode switches and view options are not creations, so they live in the title
 *    strip as icons — that is what freed the fourth grid cell for remote
 *    sessions, whose entry was previously buried in Settings.
 *
 * Search stays below the grid so creation reads as one command group before the
 * user scans existing sessions.
 */
const SessionCreateBar: React.FC<SessionCreateBarProps> = ({
  batchMode,
  onToggleBatchMode,
  onNewChat,
  onNewTerminal,
  onCreateProject,
  displayPreferences,
  onDisplayPresetChange,
  onDisplayPreferenceChange,
  onCollapse,
  onSessionClick,
  onConversationSelect,
}) => {
  const { t } = useTranslation();
  const actionButtonClassName =
    'w-full min-w-0 h-28px px-9px rd-8px border border-solid outline-none flex items-center justify-center gap-6px text-13px font-[500] leading-none transition-colors focus:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--primary-6),0.12)]';
  const restingButtonClassName =
    'cursor-pointer bg-transparent border-[var(--color-border-2)] text-t-primary hover:bg-fill-3 active:bg-fill-4';
  const batchToggleLabel = t(batchMode ? 'sessionList.exitBatchSelect' : 'sessionList.batchSelect');
  const plusIcon = (
    <Plus
      theme='outline'
      size='15'
      fill='currentColor'
      className='block leading-none shrink-0'
      style={{ lineHeight: 0 }}
    />
  );

  return (
    <div className='shrink-0 px-10px pt-8px pb-6px flex flex-col gap-6px'>
      {/* Title strip: mode switches and view options, never creations */}
      <div className='flex items-center h-20px px-2px select-none'>
        <span className='text-13px text-t-tertiary font-[500] leading-none tracking-wide'>
          {t('sessionList.title')}
        </span>
        <div className='ml-auto flex items-center gap-2px'>
          <InstantHoverTooltip content={batchToggleLabel} position='bottom'>
            <button
              type='button'
              data-testid='workpath-batch-select-btn'
              className={classNames(
                'size-22px rd-4px flex items-center justify-center cursor-pointer shrink-0 transition-colors bg-transparent border-none outline-none focus:outline-none focus-visible:outline-none',
                batchMode
                  ? 'bg-[rgba(var(--primary-6),0.12)] text-primary hover:bg-[rgba(var(--primary-6),0.18)]'
                  : 'text-t-secondary hover:text-t-primary hover:bg-fill-4'
              )}
              onClick={onToggleBatchMode}
              aria-pressed={batchMode}
              aria-label={batchToggleLabel}
            >
              <ListCheckbox theme='outline' size='14' fill='currentColor' className='block leading-none' />
            </button>
          </InstantHoverTooltip>
          <SessionDisplaySettingsPopover
            preferences={displayPreferences}
            onPresetChange={onDisplayPresetChange}
            onPreferenceChange={onDisplayPreferenceChange}
          />
          <InstantHoverTooltip content={t('sessionList.collapseList')} position='bottom'>
            <div
              data-testid='session-sider-collapse'
              className='size-22px rd-6px flex items-center justify-center cursor-pointer shrink-0 transition-colors text-t-secondary hover:text-t-primary hover:bg-fill-3'
              onClick={onCollapse}
              aria-label={t('sessionList.collapseList')}
            >
              <ExpandLeft
                theme='outline'
                size='15'
                fill='currentColor'
                className='block leading-none shrink-0'
                style={{ lineHeight: 0 }}
              />
            </div>
          </InstantHoverTooltip>
        </div>
      </div>

      {/* Creation actions only. `+` carries the verb, the label carries the noun,
          and the full phrase stays on aria-label for anyone who needs it. */}
      <div data-testid='session-action-grid' className='grid grid-cols-2 gap-4px'>
        <button
          type='button'
          data-testid='session-new-conversation-entry'
          className={classNames(actionButtonClassName, restingButtonClassName)}
          onClick={onNewChat}
          aria-label={t('terminal.newConversation')}
        >
          {plusIcon}
          <span className='truncate min-w-0'>{t('sessionList.actionChat')}</span>
        </button>
        <button
          type='button'
          data-testid='session-new-terminal-entry'
          className={classNames(actionButtonClassName, restingButtonClassName)}
          onClick={onNewTerminal}
          aria-label={t('terminal.newTerminal')}
        >
          {plusIcon}
          <span className='truncate min-w-0'>{t('sessionList.actionTerminal')}</span>
        </button>
        <button
          type='button'
          data-testid='workpath-create-project-btn'
          className={classNames(actionButtonClassName, restingButtonClassName)}
          onClick={onCreateProject}
          aria-label={t('sessionList.createProject')}
        >
          {plusIcon}
          <span className='truncate min-w-0'>{t('sessionList.actionProject')}</span>
        </button>
        <RemoteSessionPopover
          buttonClassName={classNames(actionButtonClassName, restingButtonClassName)}
          onLaunched={() => {
            onConversationSelect();
            onSessionClick?.();
          }}
        />
      </div>

      <div className='w-full'>
        <ConversationSearchPopover
          onSessionClick={onSessionClick}
          onConversationSelect={onConversationSelect}
          label={t('conversation.historySearch.shortTitle')}
          fullWidth
        />
      </div>
    </div>
  );
};

export default SessionCreateBar;
