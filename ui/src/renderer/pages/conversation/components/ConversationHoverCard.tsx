/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';

import type { TChatConversation } from '@/common/config/storage';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import type { TFunction } from 'i18next';

/**
 * Returns an i18n'd active status label for a conversation.
 * Prefers runtime.state, falls back to conversation.status, then idle.
 */
export const conversationActiveLabel = (c: TChatConversation, t: TFunction): string => {
  if (c.runtime?.state) {
    return t(`conversation.hoverCard.runtime.${c.runtime.state}`);
  }
  if (c.status) {
    return t(`conversation.hoverCard.statusLabels.${c.status}`);
  }
  return t('conversation.hoverCard.runtime.idle');
};

type ConversationHoverCardProps = {
  conversation: TChatConversation;
};

const ConversationHoverCard: React.FC<ConversationHoverCardProps> = ({ conversation }) => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-col gap-6px py-4px min-w-200px max-w-320px'>
      <div className='flex flex-col gap-2px'>
        <span className='text-12px text-t-tertiary'>{t('conversation.hoverCard.name')}</span>
        <span className='text-13px text-t-primary break-all'>
          {conversation.name || t('conversation.welcome.newConversation')}
        </span>
      </div>
      {/* Full session ID shown as text (selectable) with a copy affordance —
          surfaced so the row's short-ID chip can be resolved to the real ID for
          session location. */}
      <div className='flex flex-col gap-2px'>
        <span className='text-12px text-t-tertiary'>{t('conversation.hoverCard.id')}</span>
        <div className='flex items-center gap-6px'>
          <span className='text-13px text-t-primary break-all font-mono leading-16px'>{conversation.id}</span>
          <CopyIconButton text={conversation.id} tooltip={t('common.copyFullId')} className='shrink-0 size-18px' />
        </div>
      </div>
      <div className='flex flex-col gap-2px'>
        <span className='text-12px text-t-tertiary'>{t('conversation.hoverCard.status')}</span>
        <span className='text-13px text-t-primary'>
          {conversationActiveLabel(conversation, t)}
        </span>
      </div>
    </div>
  );
};

export default ConversationHoverCard;
