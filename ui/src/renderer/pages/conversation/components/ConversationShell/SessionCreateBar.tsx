/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { ExpandLeft, Plus, Terminal } from '@icon-park/react';
import InstantHoverTooltip from '@renderer/components/base/InstantHoverTooltip';
import { ConversationSiderActions } from '@renderer/components/layout/Sider/SiderNav';
import ConversationSearchPopover from '@renderer/pages/conversation/SessionList/ConversationSearchPopover';
import type { SidebarDisplayPreferences, SidebarDisplayPreset } from '@renderer/pages/conversation/SessionList/utils/sidebarDisplayPreferences';
import SessionDisplaySettingsPopover from './SessionDisplaySettingsPopover';

export interface SessionCreateBarProps {
  isMobile: boolean;
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
 * ({@link ContentSider}). Carries the primary create CTAs (new conversation /
 * new terminal), search, the batch-select toggle, and an in-panel collapse
 * shortcut.
 *
 * The two create actions (new conversation / new terminal) sit side by side on
 * one row, each a quiet half-width button in the same visual language as the
 * search trigger just below and the session list rows: borderless, transparent,
 * 34px tall with an 8px radius, icon + label, and a soft fill on hover
 * (--color-fill-3 / -4). No borders, brand fills, or boxes — nothing reads as a
 * jarring standout; separation comes from a small gap, not a divider.
 */
const SessionCreateBar: React.FC<SessionCreateBarProps> = ({
  isMobile,
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

  return (
    <div className='shrink-0 px-10px pt-12px pb-8px flex flex-col gap-10px'>
      {/* Title strip + secondary actions (batch / in-panel collapse) */}
      <div className='flex items-center h-22px px-2px select-none'>
        <span className='text-13px text-t-tertiary font-[500] leading-none tracking-wide'>
          {t('sessionList.title')}
        </span>
        <div className='ml-auto flex items-center gap-2px'>
          <SessionDisplaySettingsPopover
            preferences={displayPreferences}
            onPresetChange={onDisplayPresetChange}
            onPreferenceChange={onDisplayPreferenceChange}
          />
          <ConversationSiderActions
            isMobile={isMobile}
            isBatchMode={batchMode}
            collapsed={false}
            showCreate={false}
            onNewChat={onNewChat}
            onCreateProject={onCreateProject}
            onToggleBatchMode={onToggleBatchMode}
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

      {/* Create actions on one row — same quiet language as the search trigger
          below and the session list rows (borderless, transparent, soft hover
          fill, 8px radius), split into two half-width halves. */}
      <div className='flex items-stretch gap-4px'>
        <button
          type='button'
          data-testid='session-new-conversation-entry'
          className='flex-1 basis-0 min-w-0 h-34px rd-8px flex items-center justify-center gap-6px bg-transparent border-none outline-none cursor-pointer transition-colors text-t-primary hover:bg-fill-3 active:bg-fill-4 focus:outline-none focus-visible:outline-none'
          onClick={onNewChat}
        >
          <Plus
            theme='outline'
            size='16'
            fill='currentColor'
            className='block leading-none shrink-0'
            style={{ lineHeight: 0 }}
          />
          <span className='text-14px font-[500] leading-none truncate'>{t('terminal.newConversation')}</span>
        </button>
        <button
          type='button'
          data-testid='session-new-terminal-entry'
          className='flex-1 basis-0 min-w-0 h-34px rd-8px flex items-center justify-center gap-6px bg-transparent border-none outline-none cursor-pointer transition-colors text-t-primary hover:bg-fill-3 active:bg-fill-4 focus:outline-none focus-visible:outline-none'
          onClick={onNewTerminal}
        >
          <Terminal
            theme='outline'
            size='16'
            fill='currentColor'
            className='block leading-none shrink-0'
            style={{ lineHeight: 0 }}
          />
          <span className='text-14px font-[500] leading-none truncate'>{t('terminal.newTerminal')}</span>
        </button>
      </div>

      {/* Search */}
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
