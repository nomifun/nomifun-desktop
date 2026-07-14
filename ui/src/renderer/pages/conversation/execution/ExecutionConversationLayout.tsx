import { Button, Tooltip } from '@arco-design/web-react';
import { Branch } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import ChatLayout, { type ChatLayoutProps } from '../components/ChatLayout';
import ExecutionContentSwitcher from './ExecutionContentSwitcher';
import { useExecution } from './ExecutionContext';
import { EXECUTION_STATUS_META } from './executionStatusMeta';
import ExecutionTopPanel from './ExecutionTopPanel';
import PlanApprovalBanner from './PlanApprovalBanner';

/**
 * Conversation-native shell for the one AgentExecution projection.
 *
 * Every authorized locally hosted Agent can delegate through its process-issued
 * Platform Gateway capability, so the
 * execution chrome is deliberately independent of the conversation runtime.
 * Runtime-specific controls still belong to the child composer; progress,
 * decisions and lifecycle commands are available wherever an authoritative
 * ConversationExecutionLink projects an active execution.
 */
const ExecutionConversationLayout: React.FC<ChatLayoutProps> = ({ children, headerExtra, ...layoutProps }) => {
  const { t } = useTranslation();
  const execution = useExecution();
  const status = execution.detail?.execution.status ?? '';

  return (
    <ChatLayout
      {...layoutProps}
      headerExtra={
        <div className='flex items-center gap-8px'>
          {headerExtra}
          {execution.executionId && (
            <Tooltip content={t(execution.canvasOpen ? 'agentExecution.panel.collapse' : 'agentExecution.panel.open')}>
              <Button
                size='mini'
                type={execution.canvasOpen ? 'primary' : 'default'}
                aria-label={t(execution.canvasOpen ? 'agentExecution.panel.collapse' : 'agentExecution.panel.open')}
                aria-pressed={execution.canvasOpen}
                icon={<Branch theme='outline' size='14' strokeWidth={3} />}
                onClick={execution.toggleCanvas}
              />
            </Tooltip>
          )}
        </div>
      }
      workspaceCollaboration={{
        active: execution.canvasOpen,
        available: Boolean(execution.executionId),
        statusColor: EXECUTION_STATUS_META[status as keyof typeof EXECUTION_STATUS_META]?.color,
        onClick: execution.toggleCanvas,
      }}
    >
      <div className='relative flex flex-row flex-1 min-h-0' data-testid='conversation-execution-layout'>
        <div className='flex-1 min-w-0 min-h-0 flex flex-col'>
          <PlanApprovalBanner />
          <ExecutionContentSwitcher>{children}</ExecutionContentSwitcher>
        </div>
        <ExecutionTopPanel />
      </div>
    </ChatLayout>
  );
};

export default ExecutionConversationLayout;
