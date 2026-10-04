/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageToolGroup } from '@/common/chat/chatLib';
import { optionalDisplayText, toDisplayText } from '@/common/chat/displayText';
import { iconColors } from '@/renderer/styles/colors';
import { Alert, Tag } from '@arco-design/web-react';
import { LoadingOne } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import FeedbackButton from '@/renderer/components/base/FeedbackButton';

const ALERT_CLASSES =
  '!items-start !rd-8px !px-8px [&_.arco-alert-icon]:flex [&_.arco-alert-icon]:items-start [&_.arco-alert-content-wrapper]:flex [&_.arco-alert-content-wrapper]:items-start [&_.arco-alert-content-wrapper]:w-full [&_.arco-alert-content]:flex-1';

const MessageToolGroup: React.FC<{ message: IMessageToolGroup }> = ({ message }) => {
  const { t } = useTranslation();
  const tools = Array.isArray(message.content) ? message.content : [];
  return <div>{tools.map((content, index) => {
    const status = toDisplayText(content.status);
    const description = optionalDisplayText(content.description);
    const loading = status !== 'Success' && status !== 'Error' && status !== 'Canceled';
    return <div key={toDisplayText(content.call_id, `tool-${index}`)}>
      <Alert
        className={ALERT_CLASSES}
        type={status === 'Error' ? 'error' : status === 'Success' ? 'success' : status === 'Canceled' ? 'warning' : 'info'}
        icon={loading && <LoadingOne theme='outline' size='12' fill={iconColors.primary} className='loading lh-[1] flex' />}
        content={<Tag className='mr-4px'>
          {toDisplayText(content.name, 'Tool')}
          {status === 'Canceled' ? `(${t('messages.canceledExecution')})` : ''}
        </Tag>}
      />
      {(description || status === 'Error') && <div className='mt-8px'>
        {description && <div className={`text-12px text-t-secondary mb-2 ${status === 'Error' ? 'whitespace-pre-wrap break-words' : 'truncate'}`}>
          {description}
        </div>}
        {status === 'Error' && <div className='mt-4px flex justify-end'><FeedbackButton /></div>}
      </div>}
    </div>;
  })}</div>;
};

export default MessageToolGroup;
