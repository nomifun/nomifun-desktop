import React, { createContext, useContext, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId, type ConversationId, type MessageId } from '@/common/types/ids';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import { useThinkingDisplayPreferences } from '@/renderer/hooks/config/useThinkingDisplayPreferences';
import { MessageListLoadingProvider, MessageListProvider, useMessageList, useMessageLstCache } from '@/renderer/pages/conversation/Messages/hooks';
import { useNomiMessage } from '@/renderer/pages/conversation/platforms/nomi/useNomiMessage';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { ExecutionProvider, useExecutionSafe } from '@/renderer/pages/conversation/execution/ExecutionContext';
import ExecutionContentSwitcher from '@/renderer/pages/conversation/execution/ExecutionContentSwitcher';
import ProcessTraceItem from '@/renderer/pages/conversation/Messages/components/ProcessTraceItem';
import TurnProcessDisclosure from '@/renderer/pages/conversation/Messages/components/TurnProcessDisclosure';
import { ToolDetailsProvider } from '@/renderer/pages/conversation/Messages/components/ToolDetails';
import DelegationRecordRow from '@/renderer/pages/conversation/Messages/components/DelegationRecordRow';
import { useDelegationRecords } from '@/renderer/pages/conversation/Messages/useDelegationRecords';
import { insertDelegationRecords, withoutRecordedDelegationCalls } from '@/renderer/pages/conversation/Messages/delegationProcessModel';
import { getProcessItemState } from '@/renderer/pages/conversation/Messages/turnProcessState';
import { creativeAcceptedTurnOwners, creativeMessageTurn, creativeProcessOwnerIds, creativeProcessSegments } from './canonicalProcessModel';
import type { ProcessSegmentMessage } from '@/renderer/pages/conversation/Messages/processSegments';
import type { CreativeStudioAgentMessage } from './types';
import '@/renderer/pages/conversation/Messages/messages.css';

type ProcessContext = {
  render: (message: CreativeStudioAgentMessage) => React.ReactNode;
  owners: (messages: readonly CreativeStudioAgentMessage[]) => ReadonlySet<string>;
  older: React.ReactNode;
};
const Context = createContext<ProcessContext | null>(null);
export const useCreativeStudioCanonicalProcess = () => useContext(Context);

/** A single Session cache/subscription per mounted Canvas panel. It contains
 * only ephemeral canonical projections; Canvas persisted history is untouched. */
export function CreativeStudioCanonicalProcessProvider({ conversationId, children }: { conversationId?: string; children: React.ReactNode }) {
  if (!conversationId) return <>{children}</>;
  let id: ConversationId;
  try { id = parseConversationId(conversationId); } catch { return <>{children}</>; }
  return <MessageListProvider key={id} conversation={{ id }}><MessageListLoadingProvider>
    <CanonicalSession conversationId={id}>{children}</CanonicalSession>
  </MessageListLoadingProvider></MessageListProvider>;
}

function CanonicalSession({ conversationId, children }: { conversationId: ConversationId; children: React.ReactNode }) {
  const paging = useMessageLstCache(conversationId);
  const runtime = useNomiMessage(conversationId);
  const [snapshot, setSnapshot] = useState<TChatConversation | null>(null);
  const [snapshotFailed, setSnapshotFailed] = useState(false);
  const [snapshotRevision, setSnapshotRevision] = useState(0);
  useEffect(() => {
    let active = true;
    setSnapshotFailed(false);
    void getConversationOrNull(conversationId).then(value => {
      if (active) { setSnapshot(value); setSnapshotFailed(value === null); }
    }).catch(() => { if (active) setSnapshotFailed(true); });
    return () => { active = false; };
  }, [conversationId, snapshotRevision]);
  const body = <ProcessProjection conversationId={conversationId} activeTurnId={runtime.activeTurnId}
    activeRequestMessageId={runtime.activeRequestMessageId}
    activeTurnStartedAt={runtime.activeTurnStartedAt} running={runtime.executing} paging={paging}
    executionUnavailable={snapshotFailed} retryExecution={() => setSnapshotRevision(value => value + 1)}>{children}</ProcessProjection>;
  return <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', readOnly: true,
    isProcessing: runtime.executing, isTurnStateHydrated: runtime.hasHydratedRunningState,
    activeTurnId: runtime.activeTurnId, activeTurnStartedAt: runtime.activeTurnStartedAt,
    activeRequestMessageId: runtime.activeRequestMessageId, workspace: snapshot?.extra?.workspace }}>
    <ToolDetailsProvider scope={conversationId}>
      {snapshot ? <ExecutionProvider conversation={snapshot}>{body}</ExecutionProvider> : body}
    </ToolDetailsProvider>
  </ConversationProvider>;
}

function ProcessProjection({ conversationId, activeTurnId, activeRequestMessageId, activeTurnStartedAt, running, paging, executionUnavailable, retryExecution, children }: {
  conversationId: ConversationId; activeTurnId?: MessageId; activeRequestMessageId?: MessageId; activeTurnStartedAt?: number; running: boolean;
  paging: ReturnType<typeof useMessageLstCache>; executionUnavailable: boolean; retryExecution: () => void; children: React.ReactNode;
}) {
  const { t } = useTranslation();
  const canonical = useMessageList();
  const thinking = useThinkingDisplayPreferences();
  const execution = useExecutionSafe();
  const segments = useMemo(() => creativeProcessSegments(canonical, thinking.visible), [canonical, thinking.visible]);
  const roots = useMemo(() => [...new Set(canonical.flatMap(message => message.type === 'text' && message.position === 'right'
    ? [message.message_id ?? message.msg_id].filter((id): id is MessageId => Boolean(id)) : []).concat(activeRequestMessageId ? [activeRequestMessageId] : []))], [canonical, activeRequestMessageId]);
  const acceptedOwners = useMemo(() => creativeAcceptedTurnOwners(canonical, activeTurnId, activeRequestMessageId), [canonical, activeTurnId, activeRequestMessageId]);
  const delegation = useDelegationRecords(conversationId, roots, running, execution?.executionId);
  const value: ProcessContext = {
    owners: messages => creativeProcessOwnerIds(messages, canonical, activeTurnId),
    older: <>
      {executionUnavailable && <div role='status' className='turn-process-trace-detail__notice'>{t('messages.delegation.openFailed')}
        <button type='button' className='turn-process-trace-detail__more' onClick={retryExecution}>{t('messages.toolDetails.retry')}</button></div>}
      {paging.hasMore && <button type='button' className='turn-process-trace-detail__more' disabled={paging.loadingOlder}
        onClick={() => void paging.loadOlder()}>{t('messages.loadOlder', { defaultValue: 'Load earlier messages' })}</button>}
      {delegation.hasMore && <button type='button' className='turn-process-trace-detail__more' onClick={delegation.loadMore}>{t('messages.delegation.more')}</button>}
      {delegation.failed && <button type='button' className='turn-process-trace-detail__more' onClick={delegation.retry}>{t('messages.toolDetails.retry')}</button>}
    </>,
    render(message) {
      const turn = creativeMessageTurn(message, canonical, activeTurnId);
      if (!turn) return null;
      const records = delegation.records.filter(record => acceptedOwners.some(owner => owner.role === 'user' && owner.turnId === turn
        && (owner.sourceMessageIds?.includes(record.source.source_message_id) || owner.turnId === record.source.source_message_id)));
      const items = insertDelegationRecords(segments.filter(source => source.turn_id === turn).flatMap(source => {
        if (source.type !== 'tool_call' && source.type !== 'tool_group') return [source];
        return withoutRecordedDelegationCalls([source], records).map(tool => ({ ...source, ...tool } as ProcessSegmentMessage));
      }), records);
      if (!items.length) return null;
      const summary = canonical.find(source => source.turn_id === turn && source.type === 'agent_status' && source.content.turn_summary);
      const timing = summary?.type === 'agent_status' ? summary.content : undefined;
      const live = message.status === 'running' && turn === activeTurnId && running;
      const state = live ? 'running' : message.status === 'failed' ? 'failed' : message.status === 'stopped' ? 'canceled' : 'completed';
      return <TurnProcessDisclosure item={{ id: `creative-process:${turn}`, processItems: items,
        startAt: typeof timing?.started_at_ms === 'number' ? timing.started_at_ms : turn === activeTurnId && typeof activeTurnStartedAt === 'number' ? activeTurnStartedAt : NaN,
        endAt: typeof timing?.finished_at_ms === 'number' ? timing.finished_at_ms : NaN,
        state, running: live, defaultCollapsed: !live }}
        renderProcessItem={entry => entry.type === 'delegation_record' ? <DelegationRecordRow records={entry.records} /> : <ProcessTraceItem item={entry} stateOverride={live ? undefined : state} />}
        getProcessItemKey={entry => entry.id}
        getProcessItemState={entry => entry.type === 'delegation_record' ? entry.records[0].kind === 'failed' ? 'failed' : 'completed' : entry.processSegmentState ?? getProcessItemState(entry)}
        getProcessItemLayoutKind={entry => entry.type === 'delegation_record' ? 'delegation' : entry.type === 'thinking' ? 'thinking' : 'tool'}
        getProcessGroupId={entry => entry.processGroupId} />;
    },
  };
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function CreativeStudioProcessContent({ children }: { children: React.ReactNode }) {
  return <ExecutionContentSwitcher>{children}</ExecutionContentSwitcher>;
}
