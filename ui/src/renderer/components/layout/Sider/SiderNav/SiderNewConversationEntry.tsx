/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Tooltip } from '@arco-design/web-react';
import { Plus, Terminal } from '@icon-park/react';
import classNames from 'classnames';
import type { SiderTooltipProps } from '@renderer/utils/ui/siderTooltip';
import styles from '../Sider.module.css';

type SiderNewConversationEntryProps = {
  isMobile: boolean;
  collapsed: boolean;
  siderTooltipProps: SiderTooltipProps;
  onClick: () => void;
  /** New-terminal action; rendered as a divided second half of the row. */
  onNewTerminal: () => void;
};

const SiderNewConversationEntry: React.FC<SiderNewConversationEntryProps> = ({
  isMobile,
  collapsed,
  siderTooltipProps,
  onClick,
  onNewTerminal,
}) => {
  const { t } = useTranslation();
  const label = t('terminal.newConversation');
  const terminalLabel = t('terminal.newTerminal');

  if (collapsed) {
    // Collapsed: stack the two actions vertically as icon buttons.
    return (
      <div className='flex flex-col gap-4px'>
        <Tooltip {...siderTooltipProps} content={label} position='right'>
          <div
            data-testid='sider-new-conversation-entry'
            className={classNames(
              'w-full h-34px flex items-center justify-center cursor-pointer shrink-0 transition-colors text-t-primary rd-8px hover:bg-fill-3 active:bg-fill-4',
              styles.newChatTrigger
            )}
            onClick={onClick}
          >
            <Plus
              theme='outline'
              size='16'
              fill='currentColor'
              className='block leading-none'
              style={{ lineHeight: 0 }}
            />
          </div>
        </Tooltip>
        <Tooltip {...siderTooltipProps} content={terminalLabel} position='right'>
          <div
            data-testid='sider-new-terminal-entry'
            className='w-full h-34px flex items-center justify-center cursor-pointer shrink-0 transition-colors text-t-primary rd-8px hover:bg-fill-3 active:bg-fill-4'
            onClick={onNewTerminal}
          >
            <Terminal
              theme='outline'
              size='16'
              fill='currentColor'
              className='block leading-none'
              style={{ lineHeight: 0 }}
            />
          </div>
        </Tooltip>
      </div>
    );
  }

  // Expanded: "新会话 | 新终端" on one row, split by a vertical divider with a
  // subtle outer border to visually distinguish the grouped actions.
  return (
    <div
      className={classNames(
        'h-34px w-full flex items-stretch shrink-0 rd-0.5rem overflow-hidden border border-solid border-[var(--color-border-2)]',
        isMobile && 'sider-action-btn-mobile'
      )}
    >
      <Tooltip {...siderTooltipProps} content={label} position='right'>
        <div
          data-testid='sider-new-conversation-entry'
          className={classNames(
            styles.newChatTrigger,
            'flex-1 basis-0 min-w-0 flex items-center justify-center gap-6px cursor-pointer transition-all bg-transparent text-t-primary hover:bg-fill-3 active:bg-fill-4'
          )}
          onClick={onClick}
        >
          <Plus
            theme='outline'
            size='14'
            fill='currentColor'
            className={classNames('block leading-none shrink-0', styles.newChatIcon)}
            style={{ lineHeight: 0 }}
          />
          <span className='collapsed-hidden text-t-primary text-14px font-[500] leading-24px truncate'>{label}</span>
        </div>
      </Tooltip>
      <div className='w-px self-center h-18px bg-[var(--color-border-2)]' />
      <Tooltip {...siderTooltipProps} content={terminalLabel} position='right'>
        <div
          data-testid='sider-new-terminal-entry'
          className='flex-1 basis-0 min-w-0 flex items-center justify-center gap-6px cursor-pointer transition-all bg-transparent text-t-primary hover:bg-fill-3 active:bg-fill-4'
          onClick={onNewTerminal}
        >
          <Terminal
            theme='outline'
            size='14'
            fill='currentColor'
            className='block leading-none shrink-0'
            style={{ lineHeight: 0 }}
          />
          <span className='collapsed-hidden text-t-primary text-14px font-[500] leading-24px truncate'>
            {terminalLabel}
          </span>
        </div>
      </Tooltip>
    </div>
  );
};

export default SiderNewConversationEntry;
