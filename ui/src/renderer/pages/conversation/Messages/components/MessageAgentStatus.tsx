/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageAgentStatus } from '@/common/chat/chatLib';
import { toDisplayText } from '@/common/chat/displayText';
import { Badge, Typography } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';

const { Text } = Typography;

interface MessageAgentStatusProps {
  message: IMessageAgentStatus;
}

/**
 * Unified agent status message component for the native agent runtime.
 */
const MessageAgentStatus: React.FC<MessageAgentStatusProps> = ({ message }) => {
  const { t } = useTranslation();
  const { backend, status, agent_name } = message.content;
  const conversation = useConversationContextSafe();
  const backendText = toDisplayText(backend, 'agent');
  const agentNameText = toDisplayText(agent_name);

  // Historical labels come from the message or canonical conversation context, never the live runtime registry.
  const display_name =
    agentNameText ||
    conversation?.currentAgent?.label ||
    backendText.charAt(0).toUpperCase() + backendText.slice(1);

  // Lifecycle status does not carry a failure cause. The exact canonical
  // failed-Turn projection owns its error card; cancellation owns its notice.
  if (status === 'error') return null;

  const getStatusBadge = () => {
    switch (status) {
      case 'connecting':
        return <Badge status='processing' text={t('agent.status.connecting', { agent: display_name })} />;
      case 'connected':
        return <Badge status='success' text={t('agent.status.connected', { agent: display_name })} />;
      case 'authenticated':
        return <Badge status='success' text={t('agent.status.authenticated', { agent: display_name })} />;
      case 'session_active':
        return <Badge status='success' text={t('agent.status.session_active', { agent: display_name })} />;
      case 'preparing':
        return (
          <Badge
            status='processing'
            text={t('messages.processReceipt.preparingAction', { defaultValue: 'Preparing next action' })}
          />
        );
      case 'prepared':
        return (
          <Badge
            status='default'
            text={t('messages.processReceipt.preparedAction', { defaultValue: 'Prepared next action' })}
          />
        );
      default:
        return <Badge status='default' text={t('agent.status.unknown')} />;
    }
  };

  const isSuccess = status === 'connected' || status === 'authenticated' || status === 'session_active';
  const isPreparing = status === 'preparing' || status === 'prepared';

  return (
    <div
      className='agent-status-message flex items-center gap-3 p-3 rounded-lg border'
      style={{
        backgroundColor: isSuccess
            ? 'var(--color-success-light-1)'
            : isPreparing
              ? 'transparent'
            : 'var(--color-primary-light-1)',
        borderColor: isSuccess
            ? 'rgb(var(--success-3))'
            : isPreparing
              ? 'transparent'
              : 'rgb(var(--primary-3))',
        color: isSuccess
            ? 'rgb(var(--success-6))'
            : isPreparing
              ? 'var(--color-text-3)'
              : 'rgb(var(--primary-6))',
      }}
    >
      <div className='flex items-center gap-2'>
        <Text style={{ fontWeight: 'bold' }} className='capitalize'>
          {display_name}
        </Text>
      </div>

      <div className='flex-1 flex items-center gap-6px'>
        {getStatusBadge()}
      </div>
    </div>
  );
};

export default MessageAgentStatus;
