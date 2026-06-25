/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import InstantHoverTooltip from '@renderer/components/base/InstantHoverTooltip';
import { FolderPlus, ListCheckbox, Plus } from '@icon-park/react';
import classNames from 'classnames';

type ConversationSiderActionsProps = {
  isMobile: boolean;
  isBatchMode: boolean;
  collapsed: boolean;
  showCreate?: boolean;
  onNewChat: () => void;
  onCreateProject?: () => void;
  onToggleBatchMode: () => void;
};

const ConversationSiderActions: React.FC<ConversationSiderActionsProps> = ({
  isMobile,
  isBatchMode,
  collapsed,
  showCreate = true,
  onNewChat,
  onCreateProject,
  onToggleBatchMode,
}) => {
  const { t } = useTranslation();
  const batchTooltip = t('conversation.history.batchSelect');

  const renderAction = ({
    key,
    tooltip,
    active = false,
    onClick,
    icon,
  }: {
    key: string;
    tooltip: string;
    active?: boolean;
    onClick: () => void;
    icon: React.ReactNode;
  }) => (
    <InstantHoverTooltip key={key} content={tooltip} position={collapsed ? 'right' : 'bottom'}>
      <div
        data-testid={`conversation-${key}-btn`}
        aria-label={tooltip}
        className={classNames(
          'size-22px rd-4px flex items-center justify-center cursor-pointer shrink-0 transition-colors text-t-secondary hover:text-t-primary',
          isMobile && 'sider-action-icon-btn-mobile',
          active
            ? 'bg-[rgba(var(--primary-6),0.12)] border border-solid border-[rgba(var(--primary-6),0.24)] !text-primary'
            : 'hover:bg-fill-4'
        )}
        onClick={(event) => {
          event.stopPropagation();
          onClick();
        }}
      >
        {icon}
      </div>
    </InstantHoverTooltip>
  );

  const actions = [
    showCreate
      ? renderAction({
          key: 'create',
          tooltip: t('conversation.welcome.newConversation'),
          onClick: onNewChat,
          icon: <Plus theme='outline' size='14' fill='currentColor' className='block leading-none' />,
        })
      : null,
    onCreateProject
      ? renderAction({
          key: 'project',
          tooltip: t('sessionList.createProject'),
          onClick: onCreateProject,
          icon: <FolderPlus theme='outline' size='14' fill='currentColor' className='block leading-none' />,
        })
      : null,
    renderAction({
      key: 'batch',
      tooltip: batchTooltip,
      active: isBatchMode,
      onClick: onToggleBatchMode,
      icon: (
        <ListCheckbox theme='outline' size='14' className='block leading-none shrink-0' style={{ lineHeight: 0 }} />
      ),
    }),
  ].filter((action): action is React.ReactElement => action !== null);

  if (collapsed) {
    return <div className='shrink-0 flex flex-col items-center gap-2px w-full'>{actions}</div>;
  }

  return <div className='flex items-center gap-4px'>{actions}</div>;
};

export default ConversationSiderActions;
