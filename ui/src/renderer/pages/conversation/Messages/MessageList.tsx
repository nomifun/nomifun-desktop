/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  ConversationCreationTaskCards,
  useConversationCreationTaskOwnerMessageIds,
} from '@/renderer/creation/ConversationCreationTasks';
import type {
  IMessageText,
  IMessageToolCall,
  IMessageToolGroup,
  TMessage,
} from '@/common/chat/chatLib';
import { toDisplayText } from '@/common/chat/displayText';
import { normalizeToolMessages } from '@/common/chat/normalizeToolCall';
import { formatToolPresentationLabel, resolveToolPresentation } from '@/common/chat/toolPresentation';
import { mergeAgentTurnOutcome } from '@/common/chat/turnOutcome';
import type { AgentTurnOutcome } from '@/common/protocolBindings/AgentTurnOutcome';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { useThinkingDisplayPreferences } from '@/renderer/hooks/config/useThinkingDisplayPreferences';
import { iconColors } from '@/renderer/styles/colors';
import { CHAT_MESSAGE_JUMP_EVENT, type ChatMessageJumpDetail } from '@/renderer/utils/chat/chatMinimapEvents';
import { Image } from '@arco-design/web-react';
import { Down } from '@icon-park/react';
import classNames from 'classnames';
import React, { createContext, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation } from 'react-router-dom';
import { uuid } from '@renderer/utils/common';
import './messages.css';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import contentStyles from '../components/ConversationContentColumn.module.css';
import { useConversationColumnRef } from '../components/useConversationColumnRef';
import HOC from '@renderer/utils/ui/HOC';
import { useMessageList, useMessageListLoading } from './hooks';
import MessageAgentStatus from './components/MessageAgentStatus';
import MessageTips from './components/MessageTips';
import MessageText from './components/MessageText';
import MessageThinking from './components/MessageThinking';
import MessageListSkeleton from './components/MessageListSkeleton';
import TurnProcessDisclosure from './components/TurnProcessDisclosure';
import TurnProcessReceipt, { type TurnProcessReceiptIcon } from './components/TurnProcessReceipt';
import {
  buildToolReceiptRows,
  countBoundedSearchResults,
  getToolReceiptIcon,
} from './components/toolReceiptModel';
import ProcessTraceItem from './components/ProcessTraceItem';
import { ToolDetailsProvider } from './components/ToolDetails';
import DelegationRecordRow from './components/DelegationRecordRow';
import { useDelegationRecords } from './useDelegationRecords';
import { isTerminalExecutionStatus } from '../execution/executionStatusMeta';
import { insertDelegationRecords, withoutRecordedDelegationCalls, type DelegationProcessItem } from './delegationProcessModel';
import { isContextCompressionTip } from './processTipModel';
import {
  selectJournalProcessItems,
} from './processTraceDisplayModel';
import { useAutoScroll } from './useAutoScroll';
import { useAutoPreviewOfficeFiles } from '@/renderer/hooks/file/useAutoPreviewOfficeFiles';
import SelectionReplyButton from './components/SelectionReplyButton';
import ConversationQuestionLocator from '../components/ConversationTitleMinimap/ConversationQuestionLocator';
import {
  assignTurnIdsFromUserRequests,
  buildTurnDisclosureItems,
  selectTurnPublicResultIds,
  type TurnDisclosureProcessState,
  type TurnDisclosureInputItem,
  type TurnDisclosureOutputItem,
} from './turnDisclosureModel';
import { getProcessItemState } from './turnProcessState';
import { projectProcessSegments, orderProcessSegments, type ProcessSegmentIdentity } from './processSegments';
import {
  buildTurnFileItems,
  collectVerifiedTurnImages,
  type TurnImageCandidate,
  type TurnDeliverableItem,
  type TurnGateInfo,
} from './turnDeliverablesModel';
import TurnDeliverablesCard from './components/TurnDeliverablesCard';
import KnowledgeWritebackNotice from './components/KnowledgeWritebackNotice';
import { knowledgeWritebackFromMessage, mergeKnowledgeWritebackNotices, type KnowledgeWritebackNotice as KnowledgeNotice } from './knowledgeWritebackNoticeModel';
import { useTurnFiles } from './useTurnFiles';
import { isInternalInstructionToolCall, isTaskPlanControlReceipt } from './toolMessageVisibility';
import type { ConversationId, MessageId } from '@/common/types/ids';
import { creationTaskPlacementAfterIndices } from './creationTaskPlacement';
import { useExecutionSafe } from '../execution/ExecutionContext';
import { delegatedTurnPresentation, resolveConversationDelegation } from './conversationDelegationModel';
import { conversationPauseErrorMessage } from '../utils/conversationPauseError';
import { collectSystemPermissionNotices, shouldCompactPermissionBlockedSummary, type SystemPermissionNotice as PermissionNotice } from './systemPermissionNoticeModel';
import SystemPermissionNotice from './components/SystemPermissionNotice';
import { ConversationProseContext } from './components/ConversationProse';
import { proseSourceIdentity } from './proseSource';
import { useStreamingProseIngress } from './streamingProse';

type SourceMessageId = MessageId;

type IMessageVO =
  | (TMessage & ProcessSegmentIdentity)
  | {
      type: 'tool_summary';
      processGroupId?: string;
      processSequence?: number;
      processSegmentState?: TurnDisclosureProcessState;
      id: string;
      msg_id?: MessageId;
      turn_id?: MessageId;
      messages: Array<IMessageToolGroup | IMessageToolCall>;
      sourceMessageIds: SourceMessageId[];
      created_at: number;
    };
type IRenderableItem = IMessageVO;
type ITurnProcessDisclosureVO = {
  type: 'turn_process_disclosure';
  finalAnswer?: string;
  hasInterruptedReply?: boolean;
  id: string;
  msg_id: MessageId;
  processItems: IRenderableItem[];
  processItemStates: Record<string, TurnDisclosureProcessState>;
  sourceMessageIds: SourceMessageId[];
  created_at: number;
  startAt: number;
  endAt: number;
  state: TurnDisclosureProcessState;
  running: boolean;
  defaultCollapsed: boolean;
};
type IProcessReceiptVO = {
  type: 'process_receipt';
  id: string;
  msg_id?: MessageId;
  item: IRenderableItem;
  sourceMessageIds: SourceMessageId[];
  created_at: number;
  state: TurnDisclosureProcessState;
  label: string;
  icon: TurnProcessReceiptIcon;
  defaultExpanded: boolean;
  hasDetail?: boolean;
  attention?: boolean;
};
type ITurnDeliverablesVO = {
  type: 'turn_deliverables';
  id: string;
  turn_id: MessageId;
  items: TurnDeliverableItem[];
  partial: boolean;
  sourceMessageIds: SourceMessageId[];
  created_at: number;
};
type ITurnActionsVO = {
  type: 'turn_actions';
  id: string;
  turn_id: MessageId;
  message: IMessageText;
  sourceMessageIds: SourceMessageId[];
  created_at: number;
};
type ITurnKnowledgeNoticeVO = {
  type: 'turn_knowledge_writeback';
  id: string;
  turn_id: MessageId;
  notice: KnowledgeNotice;
  sourceMessageIds: SourceMessageId[];
  created_at: number;
};
type ITurnCreationTasksVO = {
  type: 'turn_creation_tasks';
  id: string;
  turn_id: MessageId;
  message_id: MessageId;
  sourceMessageIds: SourceMessageId[];
  created_at: number;
};
type ITurnPermissionVO = PermissionNotice & { type: 'turn_permission_required' };
type IProcessedItem =
  | IRenderableItem
  | ITurnProcessDisclosureVO
  | IProcessReceiptVO
  | ITurnDeliverablesVO
  | ITurnActionsVO
  | ITurnKnowledgeNoticeVO
  | ITurnCreationTasksVO
  | ITurnPermissionVO;

type ConversationLocationState = {
  targetMessageId?: MessageId;
  fromConversationSearch?: boolean;
};

const getProcessedItemSourceMessageIds = (item: IProcessedItem): SourceMessageId[] => {
  if (
    'type' in item &&
    (item.type === 'turn_process_disclosure' ||
      item.type === 'process_receipt' ||
      item.type === 'turn_deliverables' ||
      item.type === 'turn_actions' ||
      item.type === 'turn_knowledge_writeback' ||
      item.type === 'turn_permission_required' ||
      item.type === 'turn_creation_tasks')
  ) {
    return item.sourceMessageIds;
  }
  if ('type' in item && item.type === 'tool_summary') {
    return item.sourceMessageIds;
  }
  const message = item as TMessage;
  const businessId = message.message_id ?? message.msg_id;
  return businessId ? [businessId] : [];
};

const matchesTargetMessage = (item: IProcessedItem, targetMessageId?: MessageId): boolean => {
  if (!targetMessageId) {
    return false;
  }
  return getProcessedItemSourceMessageIds(item).includes(targetMessageId);
};

const getMessageBusinessIdentity = (message: TMessage): SourceMessageId | undefined =>
  message.message_id ?? message.msg_id;

const getProcessedItemAnchorId = (item: IProcessedItem): string => {
  return 'id' in item ? item.id : uuid();
};

const getProcessedItemCreatedAt = (item: IProcessedItem): number => {
  if (
    'type' in item &&
    [
      'tool_summary',
      'turn_process_disclosure',
      'process_receipt',
      'turn_deliverables',
      'turn_actions',
      'turn_knowledge_writeback',
      'turn_permission_required',
      'turn_creation_tasks',
    ].includes(item.type)
  ) {
    // `includes` doesn't narrow the union, so `created_at` is still typed
    // `number | undefined`; the synthetic VO types always carry a number, so
    // `?? 0` is a no-op fallback (mirrors the branch below).
    return item.created_at ?? 0;
  }
  return item.created_at ?? 0;
};

const getThinkingDurationMs = (item: IRenderableItem): number | undefined => {
  if (!('type' in item) || item.type !== 'thinking') return undefined;
  const duration = item.content.duration;
  if (typeof duration !== 'number' || !Number.isFinite(duration) || duration <= 0) return undefined;
  return duration;
};

const getProcessedItemProcessStartedAt = (item: IRenderableItem): number => {
  // Canonical text created_at is a Session/keyset cursor, not elapsed time.
  // Legacy messages without a separate wall clock already use created_at as one.
  const displayAt = item.type === 'text' ? item.content.display_at_ms : undefined;
  return typeof displayAt === 'number' && Number.isFinite(displayAt)
    ? displayAt
    : getProcessedItemCreatedAt(item);
};

const getProcessedItemProcessEndedAt = (item: IRenderableItem): number => {
  const startedAt = getProcessedItemProcessStartedAt(item);
  const duration = getThinkingDurationMs(item);
  if (duration === undefined) return startedAt;
  return startedAt + duration;
};

const getProcessedItemTurnStartedAt = (item: IRenderableItem): number | undefined => {
  if (item.type === 'agent_status' && item.content.turn_summary) {
    const value = item.content.started_at_ms;
    return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
  }
  if (item.type === 'tips') {
    const value = item.content.started_at_ms;
    return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
  }
  return undefined;
};

const getProcessedItemTurnEndedAt = (item: IRenderableItem): number | undefined => {
  if (item.type === 'agent_status' && item.content.turn_summary) {
    const value = item.content.finished_at_ms;
    return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
  }
  if (item.type === 'tips') {
    const value = item.content.finished_at_ms;
    return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
  }
  return undefined;
};

const isTerminalAssistantItem = (item: IRenderableItem): boolean =>
  item.type === 'tips' && item.content.type === 'error' && !item.content.idmm_notice;

const isHiddenProcessItem = (item: IRenderableItem): boolean => {
  if (item.type !== 'thinking' && item.type !== 'text') return false;
  const text = toDisplayText(item.content.content).trim();
  if (!text) return !(item.type === 'thinking' && item.processGroupId);
  return item.type === 'thinking' && /^\[Private reasoning omitted(?: from replay)?\]$/i.test(text);
};

const getProcessedItemMsgId = (item: IRenderableItem): MessageId | undefined => {
  return item.msg_id;
};

const getProcessedItemTurnId = (item: IRenderableItem): MessageId | undefined => {
  return item.turn_id;
};

const getProcessedItemRole = (item: IRenderableItem): TurnDisclosureInputItem['role'] => {
  if ('type' in item && item.type === 'tool_summary') {
    return 'process';
  }
  switch (item.type) {
    case 'text':
      if (item.position === 'left' && (item.content.agentMessage || item.content.standalone_projection)) return 'other';
      return item.position === 'right' ? 'user' : 'assistant';
    case 'tips':
      if (item.content.agent_transition || item.content.idmm_notice || item.content.query_error) return 'other';
      if (isContextCompressionTip(item)) return 'process';
      return 'assistant';
    case 'thinking':
      return 'process_content';
    case 'tool_call':
    case 'tool_group':
      return 'process';
    case 'agent_status':
      if (item.content.turn_outcome?.kind === 'unavailable') return 'assistant';
      return item.content.turn_summary ? 'metadata' : 'process';
    default:
      return 'other';
  }
};

type TranslationFn = ReturnType<typeof useTranslation>['t'];

const compactReceiptText = (value: unknown, fallback: string): string => {
  if (typeof value !== 'string') return fallback;
  const compacted = value.replace(/\s+/g, ' ').trim();
  return compacted || fallback;
};

type ProcessReceiptSummary = {
  label: string;
  icon: TurnProcessReceiptIcon;
  defaultExpanded: boolean;
  hasDetail?: boolean;
  attention?: boolean;
};

const buildProcessReceiptSummary = (
  item: IRenderableItem,
  state: TurnDisclosureProcessState,
  t: TranslationFn,
  workspaceRoots: string[] = [],
  options: { language?: string } = {}
): ProcessReceiptSummary => {
  if ('type' in item && item.type === 'tool_summary') {
    const tools = normalizeToolMessages(item.messages);
    const receiptRows = buildToolReceiptRows(tools, options.language);
    const summarySeparator = t('messages.processReceipt.summarySeparator', { defaultValue: ', ' });
    const boundedSearchCount = countBoundedSearchResults(tools);
    const label = boundedSearchCount > 0
        ? t('messages.processReceipt.searchResultsLimited', {
            defaultValue: 'Search results limited; full reconciliation required',
          })
      : receiptRows.length
        ? tools.map(tool => formatToolPresentationLabel(resolveToolPresentation(tool, options.language))).join(summarySeparator)
        : t('messages.processReceipt.tools', {
            count: item.messages.length,
            defaultValue: '{{count}} tools',
          });
    return {
      label,
      icon: getToolReceiptIcon(receiptRows) ?? 'tool',
      defaultExpanded: false,
      hasDetail: true,
      ...(boundedSearchCount > 0 ? { attention: true } : {}),
    };
  }

  switch (item.type) {
    case 'agent_status':
      return {
        label:
item.content.status === 'preparing'
            ? t('messages.processReceipt.preparingAction', { defaultValue: 'Preparing next action' })
            : item.content.status === 'prepared'
              ? t('messages.processReceipt.preparedAction', { defaultValue: 'Prepared next action' })
              : state === 'failed'
                ? t('messages.processReceipt.agentFailed', {
                    target: item.content.agent_name || item.content.backend,
                    defaultValue: '{{target}} failed',
                  })
                : t('messages.processReceipt.agentConnecting', {
                    target: item.content.agent_name || item.content.backend,
                    defaultValue: 'Connecting {{target}}',
                  }),
        icon: 'status',
        defaultExpanded: false,
        hasDetail: false,
      };
    case 'tips':
      if (isContextCompressionTip(item)) {
        return {
          label: t('messages.processReceipt.contextCompressed', { defaultValue: 'Context compressed' }),
          icon: 'status',
          defaultExpanded: false,
          hasDetail: false,
        };
      }
      return {
        label: compactReceiptText(
              item.content.content,
              t('messages.processReceipt.status', { target: t('messages.processing'), defaultValue: '{{target}}' })
            ),
        icon: 'status',
        defaultExpanded: state === 'failed',
        hasDetail: false,
      };
    case 'tool_call':
    case 'tool_group':
      return buildProcessReceiptSummary(
        {
          type: 'tool_summary',
          id: `tool-summary-${item.id}`,
          msg_id: item.msg_id,
          messages: [item],
          sourceMessageIds: getProcessedItemSourceMessageIds(item),
          created_at: item.created_at ?? 0,
        },
        state,
        t,
        workspaceRoots,
        options
      );
    default:
      return {
        label: t('messages.processReceipt.status', {
              target: t('messages.processing'),
              defaultValue: '{{target}}',
            }),
        icon: 'status',
        defaultExpanded: false,
        hasDetail: false,
      };
  }
};

const highlightStyle: React.CSSProperties = {
  backgroundColor: 'var(--aou-1)',
  boxShadow: '0 0 0 1px var(--aou-6) inset',
  borderRadius: '12px',
};

const getUnhandledMessageType = (_message: never): string => 'unknown';

/** Scroll-up zone (px from top) that triggers loading the next older window. */
const TOP_LOAD_THRESHOLD_PX = 96;

// Image preview context
export const ImagePreviewContext = createContext<{ inPreviewGroup: boolean }>({ inPreviewGroup: false });

const renderProcessTraceItem = (
  item: IRenderableItem,
  workspaceRoots: string[] = [],
  stateOverride?: TurnDisclosureProcessState
) => (
  <ProcessTraceItem
    item={item}
    workspaceRoots={workspaceRoots}
    stateOverride={stateOverride}
  />
);

const getProcessItemLayoutKind = (item: IRenderableItem): string => {
  if ('type' in item && item.type === 'text') return 'text';
  if ('type' in item && item.type === 'thinking') return 'thinking';
  if (
    'type' in item &&
    ['tool_summary', 'tool_call', 'tool_group'].includes(item.type)
  ) {
    return 'tool';
  }
  if ('type' in item && (item.type === 'agent_status' || item.type === 'tips')) return 'status';
  return 'other';
};

const isPrivateJournalActivity = (item: IRenderableItem): boolean =>
  item.type === 'agent_status' &&
    item.content.turn_summary !== true &&
    (item.content.status === 'preparing' || item.content.status === 'prepared');

const MessageItem: React.FC<{ message: TMessage; highlighted?: boolean; hideActions?: boolean; compactError?: boolean }> = React.memo(
  HOC((props) => {
    const { message, highlighted } = props as { message: TMessage; highlighted?: boolean; hideActions?: boolean };
    return (
      <div
        id={`message-${message.id}`}
        data-message-business-id={message.message_id ?? message.msg_id}
        data-testid={`message-${message.type}-${message.position}`}
        data-message-type={message.type}
        data-message-position={message.position}
        className={classNames(
          'min-w-0 flex items-start message-item [&>div]:max-w-full',
          message.type,
          {
            'm-t-6px': message.type === 'tips' && message.content.type === 'error',
            'm-t-10px': message.type !== 'tips' || message.content.type !== 'error',
            'justify-center': message.position === 'center',
            'justify-end': message.position === 'right',
            'justify-start': message.position === 'left',
          }
        )}
        style={highlighted ? highlightStyle : undefined}
      >
        {props.children}
      </div>
    );
  })(({ message, hideActions, compactError }) => {
    const { t } = useTranslation();
    switch (message.type) {
      case 'text':
        return <MessageText message={message} hideActions={hideActions}></MessageText>;
      case 'tips':
        return <MessageTips message={message} compactError={compactError}></MessageTips>;
      case 'tool_call':
      case 'tool_group':
        return <ProcessTraceItem item={message} />;
      case 'agent_status':
        if (message.content.turn_outcome?.kind === 'unavailable') {
          return <div role='note' data-testid='turn-outcome-unavailable' className='text-t-secondary text-13px'>
            {t('messages.turnOutcomeUnavailable')}
          </div>;
        }
        return <MessageAgentStatus message={message}></MessageAgentStatus>;
      case 'thinking':
        return <MessageThinking message={message}></MessageThinking>;
      default:
        return <div>{t('messages.unknownMessageType', { type: getUnhandledMessageType(message) })}</div>;
    }
  }),
  (prev, next) =>
    prev.message.id === next.message.id &&
    prev.message.content === next.message.content &&
    prev.message.position === next.message.position &&
    prev.message.type === next.message.type &&
    prev.highlighted === next.highlighted &&
    prev.hideActions === next.hideActions &&
    prev.compactError === next.compactError
);

const MessageList: React.FC<{
  className?: string;
  emptySlot?: React.ReactNode;
  /** Live first-submit echo only; never added to canonical history or Turn state. */
  pendingMessage?: IMessageText;
  /** The real progress/result row has committed, so the submit echo can retire. */
  onPendingMessageReady?: () => void;
  /** Windowed-history paging (nomi surfaces): prepend the next older message
   *  window when the user scrolls to the top. Omitted on chats that still load
   *  their whole transcript at once. */
  onLoadOlder?: () => void | boolean | Promise<void | boolean>;
  hasMoreOlder?: boolean;
  loadingOlder?: boolean;
  /** Product opt-in; preserve the latest reply across automatic process collapse. */
  followTurnCompletion?: boolean;
}> = ({ emptySlot, pendingMessage, onPendingMessageReady, onLoadOlder, hasMoreOlder, loadingOlder, followTurnCompletion = false }) => {
  const list = useMessageList();
  const proseIngress = useStreamingProseIngress();
  const isMessageListLoading = useMessageListLoading();
  const conversationContext = useConversationContextSafe();
  const pauseError = useMemo(() => {
    const pause = conversationContext?.executionPause;
    if (!pause || !conversationContext?.conversation_id) return null;
    // A canonical error closes this Turn. A stale pause snapshot must not
    // mask that failure while its lifecycle notification is being delivered.
    const alreadyShown = list.some(message => message.type === 'tips' && message.content.type === 'error'
      && message.turn_id === pause.turnId && message.content.error
      && !message.content.idmm_notice);
    return alreadyShown ? null : conversationPauseErrorMessage(conversationContext.conversation_id, pause);
  }, [list, conversationContext?.executionPause, conversationContext?.conversation_id]);
  const execution = useExecutionSafe();
  const creationTaskOwnerMessageIds = useConversationCreationTaskOwnerMessageIds();
  const thinkingDisplay = useThinkingDisplayPreferences();
  useAutoPreviewOfficeFiles(conversationContext);
  const workspaceRoots = useMemo(
    () => (conversationContext?.workspace ? [conversationContext.workspace] : []),
    [conversationContext?.workspace]
  );
  const { t, i18n } = useTranslation();
  const location = useLocation();
  const toolLanguage = i18n.resolvedLanguage ?? i18n.language;
  const locationState = (location.state || {}) as ConversationLocationState;
  const targetMessageId = locationState.targetMessageId;
  const [highlightedMessageId, setHighlightedMessageId] = useState<MessageId | undefined>();
  const handledTargetKeyRef = useRef<string>('');
  const [pendingQuestionJump, setPendingQuestionJump] = useState<ChatMessageJumpDetail | null>(null);
  const [loadingQuestionPage, setLoadingQuestionPage] = useState(false);

  // Pre-process message list to group tool outputs into summary cards
  const processedList = useMemo(() => {
    // Canonical content references own reading boundaries. Routine model
    // lifecycle activity does not introduce another content boundary.
    const journalSources = selectJournalProcessItems(list, {
      running: conversationContext?.isProcessing === true,
      isPrivateActivity: isPrivateJournalActivity,
      isRunning: (message) => getProcessItemState(message) === 'running',
    });
    const result: Array<IMessageVO> = [];
    const publicSources = new Set<string>();
    const outcomes = new Map<MessageId, AgentTurnOutcome>();
    for (const message of list) {
      const outcome = message.type === 'tips' || message.type === 'agent_status' ? message.content.turn_outcome : undefined;
      if (outcome && message.turn_id) outcomes.set(message.turn_id, mergeAgentTurnOutcome(outcomes.get(message.turn_id), outcome)!);
    }
    for (const outcome of outcomes.values()) if ('message_ids' in outcome) for (const id of outcome.message_ids) publicSources.add(id);
    for (const message of projectProcessSegments(journalSources, publicSources)) {
      if (message.hidden || isInternalInstructionToolCall(message) || isTaskPlanControlReceipt(message)) continue;
      if (message.type === 'agent_status' && ['connecting', 'connected', 'authenticated', 'session_active'].includes(message.content.status)) continue;
      if (message.type === 'thinking' && !thinkingDisplay.visible) continue;
      if (message.type === 'tool_call' || message.type === 'tool_group') {
        const businessId = getMessageBusinessIdentity(message);
        result.push({ type: 'tool_summary', id: message.id, msg_id: message.msg_id, turn_id: message.turn_id,
          messages: [message], sourceMessageIds: businessId ? [businessId] : [], created_at: message.created_at ?? 0,
          processGroupId: message.processGroupId, processSequence: message.processSequence, processSegmentState: message.processSegmentState });
      } else result.push(message);
    }
    return result;
  }, [list, thinkingDisplay.visible, conversationContext?.isProcessing]);

  const delegationRoots = useMemo(() => [...list.flatMap(message => message.type === 'text' && message.position === 'right' && message.msg_id ? [message.msg_id] : []),
    ...(conversationContext?.activeRequestMessageId ? [conversationContext.activeRequestMessageId] : [])], [list, conversationContext?.activeRequestMessageId]);
  const delegationRecords = useDelegationRecords(conversationContext?.conversation_id, delegationRoots,
    conversationContext?.isProcessing === true || Boolean(execution?.executionId && !isTerminalExecutionStatus(execution.detail?.execution.status)), execution?.executionId);


  const delegationOwners = useMemo(() => assignTurnIdsFromUserRequests(list.map(message => {
    const role = getProcessedItemRole(message);
    return { id: message.id, turnId: role === 'user' ? message.msg_id : message.turn_id, role,
      createdAt: message.created_at ?? 0, displayAt: message.type === 'text' ? message.content.display_at_ms : undefined,
      sourceMessageIds: getProcessedItemSourceMessageIds(message), turnStartedAt: getProcessedItemTurnStartedAt(message) };
  }), { activeTurnId: conversationContext?.activeTurnId, activeRequestMessageId: conversationContext?.activeRequestMessageId }),
  [list, conversationContext?.activeTurnId, conversationContext?.activeRequestMessageId]);
  const delegation = useMemo(() => resolveConversationDelegation(conversationContext?.conversation_id, execution,
    delegationOwners, delegationRecords.records, { turnId: conversationContext?.activeTurnId, requestMessageId: conversationContext?.activeRequestMessageId }),
    [conversationContext?.conversation_id, conversationContext?.activeTurnId, conversationContext?.activeRequestMessageId, execution, delegationOwners, delegationRecords.records]);


  const pausedFileTurnId = useMemo(() => {
    const pausedTurnId = conversationContext?.executionPause?.turnId;
    if (!pausedTurnId) return undefined;
    // A pause is not a closed Turn. Only typed terminal metadata can override
    // a stale pause projection; provisional text and tail position cannot.
    const finished = list.some((message) => message.turn_id === pausedTurnId && getMessageBusinessIdentity(message) &&
      getProcessedItemTurnEndedAt(message) !== undefined);
    return finished ? undefined : pausedTurnId;
  }, [conversationContext?.executionPause?.turnId, list]);

  const turnPresentation = useMemo(() => {
    const itemById = new Map<string, IRenderableItem>();
    const rawModelInput: TurnDisclosureInputItem[] = processedList.map((item) => {
      const id = getProcessedItemAnchorId(item);
      const role = getProcessedItemRole(item);
      itemById.set(id, item);
      return {
        id,
        turnId: role === 'user' ? getProcessedItemMsgId(item) : getProcessedItemTurnId(item),
        role,
        createdAt: getProcessedItemCreatedAt(item),
        displayAt: item.type === 'text' ? item.content.display_at_ms : undefined,
        processState: getProcessItemState(item),
        processStartedAt: getProcessedItemProcessStartedAt(item),
        processEndedAt: getProcessedItemProcessEndedAt(item),
        turnStartedAt: getProcessedItemTurnStartedAt(item),
        turnEndedAt: getProcessedItemTurnEndedAt(item),
        terminal: isTerminalAssistantItem(item),
        sourceMessageIds: getProcessedItemSourceMessageIds(item),
        publicText: item.type === 'text' && item.position === 'left',
        turnOutcome: item.type === 'tips' || item.type === 'agent_status' ? item.content.turn_outcome : undefined,
        userInputRequestId: item.type === 'tips' ? item.content.user_input_request?.request_id : undefined,
      };
    });
    const modelInput = assignTurnIdsFromUserRequests(rawModelInput, {
      activeTurnId: conversationContext?.activeTurnId,
      activeRequestMessageId: conversationContext?.activeRequestMessageId,
    });
    const creationOwnerMessageIdByTurn = new Map<MessageId, MessageId>();
    for (const entry of modelInput) {
      if (!entry.turnId || entry.role !== 'user') continue;
      const ownerMessageId = entry.sourceMessageIds?.find(messageId =>
        creationTaskOwnerMessageIds.has(messageId)
      );
      if (ownerMessageId) creationOwnerMessageIdByTurn.set(entry.turnId, ownerMessageId);
    }
    const publicResultIds = selectTurnPublicResultIds(modelInput);
    const publicAssistantTextByTurn = new Map<MessageId, IMessageText>();
    for (const entry of modelInput) {
      if (!entry.turnId || !publicResultIds.has(entry.id)) continue;
      const item = itemById.get(entry.id);
      if (item?.type !== 'text' || item.position !== 'left') continue;
      // Footer/copy placement follows the public membership already resolved
      // from the receipt; an empty/unreferenced tail cannot replace it.
      publicAssistantTextByTurn.set(entry.turnId, item);
    }

    const disclosureItems = buildTurnDisclosureItems(modelInput, {
      processRecordTurnIds: new Set(modelInput.flatMap(owner => owner.role === 'user' && owner.turnId
        && delegationRecords.records.some(record => owner.sourceMessageIds?.includes(record.source.source_message_id)
          || owner.turnId === record.source.source_message_id) ? [owner.turnId] : [])),
      // Initial false is not idle authority while the runtime query is pending.
      tailClosed: conversationContext?.isTurnStateHydrated === false
        ? undefined
        : conversationContext?.isProcessing !== true,
      activeTurnId: conversationContext?.activeTurnId,
      activeTurnStartedAt: conversationContext?.activeTurnStartedAt,
      stopNotice: conversationContext?.stopNotice ?? undefined,
    })
      .map<IProcessedItem | undefined>((entry: TurnDisclosureOutputItem) => {
        if (entry.type === 'item') {
          return itemById.get(entry.id);
        }

        if (entry.type === 'process_receipt') {
          const item = itemById.get(entry.itemId);
          if (!item) return undefined;
          const state = getProcessItemState(item);
          const summary = buildProcessReceiptSummary(item, state, t, workspaceRoots, { language: toolLanguage });
          return {
            type: 'process_receipt',
            id: entry.id,
            msg_id: getProcessedItemMsgId(item),
            item,
            sourceMessageIds: getProcessedItemSourceMessageIds(item),
            created_at: getProcessedItemCreatedAt(item),
            state,
            label: summary.label,
            icon: summary.icon,
            defaultExpanded: summary.defaultExpanded,
            hasDetail: summary.hasDetail,
            attention: summary.attention,
          };
        }

        const finalTextMessage = publicAssistantTextByTurn.get(entry.turnId);
        const finalAnswer = finalTextMessage && !entry.processItemIds.includes(getProcessedItemAnchorId(finalTextMessage))
          ? toDisplayText(finalTextMessage.content.content)
          : undefined;
        const processItems = entry.processItemIds
          .map((id) => itemById.get(id))
          .filter((item): item is IRenderableItem =>
            item !== undefined &&
            !isHiddenProcessItem(item)
          );

        return {
          type: 'turn_process_disclosure',
          finalAnswer,
          hasInterruptedReply: entry.state === 'canceled' && !entry.running && Boolean(finalAnswer?.trim()),
          id: entry.id,
          msg_id: entry.turnId,
          processItems,
          processItemStates: entry.processItemStates,
          sourceMessageIds: entry.sourceMessageIds,
          created_at: entry.endAt,
          startAt: entry.startAt,
          endAt: entry.endAt,
          state: entry.state,
          running: entry.running,
          defaultCollapsed: entry.defaultCollapsed,
        };
      })
      .filter((item): item is IProcessedItem => Boolean(item));

    const turnGates = new Map<string, TurnGateInfo>();
    for (const entry of disclosureItems) {
      if ('type' in entry && entry.type === 'turn_process_disclosure') {
        turnGates.set(entry.msg_id, { running: entry.running, state: entry.state });
      }
    }
    const fileTurnGates = new Map([...turnGates].filter(([id]) => id !== pausedFileTurnId));
    const imageCandidates: TurnImageCandidate[] = [];
    for (const entry of modelInput) {
      const item = itemById.get(entry.id);
      if (item?.type === 'tool_summary') imageCandidates.push({ turnId: entry.turnId, toolMessages: item.messages });
    }
    const verifiedImagesByTurn = collectVerifiedTurnImages(imageCandidates, fileTurnGates, workspaceRoots);
    const knowledgeNoticesByTurn = new Map<MessageId, KnowledgeNotice>();
    for (const entry of modelInput) {
      if (!entry.turnId || fileTurnGates.get(entry.turnId)?.running !== false) continue;
      const item = itemById.get(entry.id);
      if (!item) continue;
      const messages = item.type === 'tool_summary' ? item.messages : [item];
      for (const message of messages) {
        const notice = knowledgeWritebackFromMessage(message);
        if (notice) knowledgeNoticesByTurn.set(entry.turnId,
          mergeKnowledgeWritebackNotices(knowledgeNoticesByTurn.get(entry.turnId), notice));
      }
    }
    return { disclosureItems, modelInput, turnGates, fileTurnGates, verifiedImagesByTurn, creationOwnerMessageIdByTurn, publicAssistantTextByTurn, knowledgeNoticesByTurn };
  }, [
    conversationContext?.activeRequestMessageId,
    conversationContext?.activeTurnId,
    conversationContext?.activeTurnStartedAt,
    conversationContext?.isTurnStateHydrated,
    conversationContext?.isProcessing,
    conversationContext?.stopNotice,
    creationTaskOwnerMessageIds,
    delegationRecords.records,
    processedList,
    pausedFileTurnId,
    t,
    toolLanguage,
    workspaceRoots,
  ]);

  const prosePresentation = useMemo(() => {
    const sources = new Map<string, string>();
    const suppressed: string[] = [];
    for (const message of list) {
      if (message.type !== 'text') continue;
      const identity = proseSourceIdentity(message);
      if (!identity) continue;
      if (message.hidden || message.content.timeline?.segments.length === 0) suppressed.push(identity);
      else sources.set(identity, message.content.content);
    }
    return { sources, suppressed, turns: turnPresentation.turnGates, pausedTurnId: pausedFileTurnId,
      liveGeneration: proseIngress.generationFor(list) };
  }, [list, turnPresentation.turnGates, pausedFileTurnId, proseIngress]);
  useLayoutEffect(() => {
    // Advance collapsed live sources as well. Reopening an old process
    // cannot replay a backlog that arrived while its body was unmounted.
    for (const identity of prosePresentation.suppressed) proseIngress.revoke(identity);
    proseIngress.commitSources(prosePresentation.sources, prosePresentation.liveGeneration);
  }, [proseIngress, prosePresentation]);

  const closedTurnIds = useMemo(() => [...turnPresentation.fileTurnGates]
    .filter(([, gate]) => !gate.running)
    .map(([turnId]) => turnId as MessageId), [turnPresentation.fileTurnGates]);
  const turnFileSummaries = useTurnFiles(conversationContext?.conversation_id, closedTurnIds);
  const canonicalPermissionTurnId = useCallback((turnId: MessageId | undefined) =>
    turnId && turnId === conversationContext?.activeRequestMessageId && conversationContext?.activeTurnId
      ? conversationContext.activeTurnId : turnId,
    [conversationContext?.activeRequestMessageId, conversationContext?.activeTurnId]);
  const permissionMessageSources = useMemo(() => {
    const associatedTurns = new Map<MessageId, MessageId>();
    for (const entry of turnPresentation.modelInput) {
      const turnId = canonicalPermissionTurnId(entry.turnId);
      if (turnId) for (const sourceId of entry.sourceMessageIds ?? []) associatedTurns.set(sourceId, turnId);
    }
    return list.map(message => {
      const sourceId = getMessageBusinessIdentity(message);
      const turnId = canonicalPermissionTurnId(message.turn_id) ?? (sourceId ? associatedTurns.get(sourceId) : undefined);
      return turnId && turnId !== message.turn_id ? { ...message, turn_id: turnId } : message;
    });
  }, [list, turnPresentation.modelInput, canonicalPermissionTurnId]);
  const permissionNotices = useMemo(() => collectSystemPermissionNotices(permissionMessageSources,
    conversationContext?.activeTurnId ?? canonicalPermissionTurnId(turnPresentation.modelInput.findLast(entry => entry.role === 'user')?.turnId)),
    [permissionMessageSources, turnPresentation.modelInput, conversationContext?.activeTurnId, canonicalPermissionTurnId]);

  const displayList = useMemo<IProcessedItem[]>(() => {
    const { disclosureItems, modelInput, turnGates, fileTurnGates, verifiedImagesByTurn, creationOwnerMessageIdByTurn, publicAssistantTextByTurn, knowledgeNoticesByTurn } = turnPresentation;
    const deliverablesByTurn = new Map<MessageId, TurnDeliverableItem[]>();
    for (const [id, gate] of fileTurnGates) {
      if (gate.running) continue;
      const turnId = id as MessageId;
      const summary = turnFileSummaries.get(turnId);
      const items = buildTurnFileItems(summary, verifiedImagesByTurn.get(turnId));
      if (items.length || summary?.complete === false) deliverablesByTurn.set(turnId, items);
    }
    const turnIdByAnchorId = new Map<string, MessageId | undefined>();
    for (const entry of modelInput) turnIdByAnchorId.set(entry.id, entry.turnId);
    const getDisplayItemTurnId = (entry: IProcessedItem): MessageId | undefined => {
      if ('type' in entry && entry.type === 'turn_process_disclosure') return entry.msg_id;
      if ('type' in entry && entry.type === 'process_receipt') return undefined;
      if ('type' in entry && entry.type === 'turn_deliverables') return entry.turn_id;
      if ('type' in entry && entry.type === 'turn_actions') return entry.turn_id;
      if ('type' in entry && entry.type === 'turn_knowledge_writeback') return entry.turn_id;
      if ('type' in entry && entry.type === 'turn_creation_tasks') return entry.turn_id;
      if ('type' in entry && entry.type === 'turn_permission_required') return entry.turn_id;
      return turnIdByAnchorId.get(getProcessedItemAnchorId(entry));
    };

    const lastIndexByTurn = new Map<MessageId, number>();
    const permissionIndexByTurn = new Map<MessageId, number>();
    disclosureItems.forEach((entry, index) => {
      const turnId = getDisplayItemTurnId(entry);
      if (turnId && (deliverablesByTurn.has(turnId) || knowledgeNoticesByTurn.has(turnId))) lastIndexByTurn.set(turnId, index);
      const permissionTurnId = canonicalPermissionTurnId(turnId ?? (entry.type === 'process_receipt'
        ? turnIdByAnchorId.get(getProcessedItemAnchorId(entry.item)) : undefined));
      if (permissionTurnId && permissionNotices.some(notice => notice.turn_id === permissionTurnId)) {
        permissionIndexByTurn.set(permissionTurnId, index);
      }
    });

    const creationTaskPlacements = creationTaskPlacementAfterIndices(
      disclosureItems.map(getDisplayItemTurnId),
      creationOwnerMessageIdByTurn
    );
    if (deliverablesByTurn.size === 0 && creationTaskPlacements.size === 0 && knowledgeNoticesByTurn.size === 0 &&
      permissionNotices.length === 0) return disclosureItems;

    // A turn's products belong inside its response, before the shared copy /
    // time footer. Decorate files and media together so neither can land after
    // turn_actions, and keep the original user bubble's actions independent.
    const withOutputs: IProcessedItem[] = [];
    disclosureItems.forEach((entry, index) => {
      withOutputs.push(entry);
      for (const notice of permissionNotices) {
        if (permissionIndexByTurn.get(notice.turn_id) === index) {
          withOutputs.push({ ...notice, type: 'turn_permission_required' });
        }
      }
      const turnId = getDisplayItemTurnId(entry);
      if (!turnId) return;
      const items = lastIndexByTurn.get(turnId) === index ? deliverablesByTurn.get(turnId) : undefined;
      const placement = creationTaskPlacements.get(index);
      const knowledgeNotice = lastIndexByTurn.get(turnId) === index ? knowledgeNoticesByTurn.get(turnId) : undefined;
      if (!items && !placement && !knowledgeNotice) return;
      if (items) {
        withOutputs.push({
          type: 'turn_deliverables',
          id: `turn-deliverables-${turnId}`,
          turn_id: turnId,
          items,
          partial: turnFileSummaries.get(turnId)?.complete === false,
          sourceMessageIds: Array.from(
            new Set(items.flatMap((item) => item.sourceMessageIds))
          ),
          created_at: getProcessedItemCreatedAt(entry),
        });
      }
      if (placement) {
        withOutputs.push({
          type: 'turn_creation_tasks',
          id: `turn-creation-tasks-${placement.turnId}`,
          turn_id: placement.turnId,
          message_id: placement.messageId,
          sourceMessageIds: [placement.messageId],
          created_at: getProcessedItemCreatedAt(entry),
        });
      }
      if (knowledgeNotice) {
        withOutputs.push({
          type: 'turn_knowledge_writeback', id: `turn-knowledge-writeback-${turnId}`,
          turn_id: turnId, notice: knowledgeNotice,
          sourceMessageIds: getProcessedItemSourceMessageIds(entry),
          created_at: getProcessedItemCreatedAt(entry),
        });
      }
      const actionMessage = publicAssistantTextByTurn.get(turnId);
      const actionMessageId = actionMessage ? getMessageBusinessIdentity(actionMessage) : undefined;
      if (actionMessage && turnGates.get(turnId)?.running !== true) {
        withOutputs.push({
          type: 'turn_actions',
          id: `turn-actions-${turnId}`,
          turn_id: turnId,
          message: actionMessage,
          sourceMessageIds: actionMessageId ? [actionMessageId] : [],
          created_at: actionMessage.created_at ?? getProcessedItemCreatedAt(entry),
        });
      }
    });
    return withOutputs;
  }, [turnPresentation, turnFileSummaries, permissionNotices, canonicalPermissionTurnId]);

  const lastUserTextIndex = useMemo(
    () =>
      displayList.findLastIndex(
        (item) =>
          !('type' in item &&
            ['turn_process_disclosure', 'process_receipt'].includes(item.type)) &&
          (item as TMessage).type === 'text' &&
          (item as TMessage).position === 'right'
      ),
    [displayList]
  );

  const isActiveProcessTextItem = useCallback(
    (item: IProcessedItem, index: number): boolean =>
      conversationContext?.isProcessing === true &&
      index > lastUserTextIndex &&
      !('type' in item &&
        ['turn_process_disclosure', 'process_receipt'].includes(item.type)) &&
      (item as TMessage).type === 'text' &&
      (item as TMessage).position === 'left',
    [conversationContext?.isProcessing, lastUserTextIndex]
  );
  const movedActionMessageIds = useMemo(
    () =>
      new Set(
        displayList
          .filter((item): item is ITurnActionsVO => 'type' in item && item.type === 'turn_actions')
          .map((item) => item.message.id)
      ),
    [displayList]
  );

  const followedTurn = followTurnCompletion
    ? displayList.findLast((item): item is ITurnProcessDisclosureVO => item.type === 'turn_process_disclosure')
    : undefined;

  // The rendered Turn already resolves canonical lifecycle and history state.
  // Reuse it for scrolling rather than keeping another completion listener.
  const {
    handleScrollerRef,
    handleContentRef,
    handleScroll,
    handleWheel,
    handlePointerDown,
    handleKeyDown,
    showScrollButton,
    scrollToBottom,
    scrollElementIntoView,
    hideScrollButton,
  } = useAutoScroll({
    messages: list,
    itemCount: displayList.length + (pauseError ? 1 : 0) + (conversationContext?.executionWait ? 1 : 0),
    followTurn: followedTurn ? { id: followedTurn.msg_id, running: followedTurn.running } : undefined,
  });
  const handleColumnRef = useConversationColumnRef(handleContentRef);

  // ── Windowed history: load older messages on scroll-up with a scroll-anchor ──
  const scrollerElRef = useRef<HTMLDivElement | null>(null);
  const lastScrollTopRef = useRef(0);
  // Set when a load-older was triggered; the layout effect below restores the
  // viewport once the prepend grows the content so the position doesn't jump.
  const prependAnchorRef = useRef<{ height: number; top: number } | null>(null);

  const handleScrollWithPaging = useCallback(
    (e: React.UIEvent<HTMLDivElement>) => {
      const el = e.currentTarget;
      scrollerElRef.current = el;
      handleScroll(e);
      const prevTop = lastScrollTopRef.current;
      lastScrollTopRef.current = el.scrollTop;
      // Fire only while actively scrolling UP into the top zone. The initial
      // mount auto-scroll-to-bottom moves scrollTop downward, so it can't trip
      // this; `prependAnchorRef` guards against re-entrancy mid-load.
      if (
        onLoadOlder &&
        hasMoreOlder &&
        !loadingOlder &&
        !prependAnchorRef.current &&
        el.scrollTop <= TOP_LOAD_THRESHOLD_PX &&
        prevTop > el.scrollTop
      ) {
        prependAnchorRef.current = { height: el.scrollHeight, top: el.scrollTop };
        void onLoadOlder();
      }
    },
    [handleScroll, onLoadOlder, hasMoreOlder, loadingOlder]
  );

  // Restore the viewport after an older window prepends (content grew at the
  // top). Keyed on the raw `list.length` (always grows by the prepended count,
  // even when the grouping transform merges cards). `overflowAnchor: none` on
  // the scroller keeps the browser from fighting this. Only acts while a
  // load-older is pending; ordinary bottom growth (streaming) leaves the anchor
  // null and is untouched.
  useLayoutEffect(() => {
    const anchor = prependAnchorRef.current;
    if (!anchor) return;
    const el = scrollerElRef.current;
    if (el) {
      const delta = el.scrollHeight - anchor.height;
      if (delta > 0) {
        el.scrollTop = anchor.top + delta;
        lastScrollTopRef.current = el.scrollTop;
      }
    }
    prependAnchorRef.current = null;
  }, [list.length]);

  useEffect(() => {
    if (!targetMessageId || displayList.length === 0) {
      return;
    }

    const targetKey = `${location.key}:${targetMessageId}`;
    if (handledTargetKeyRef.current === targetKey) {
      return;
    }

    const targetIndex = displayList.findIndex((item) => matchesTargetMessage(item, targetMessageId));
    if (targetIndex === -1) {
      if (hasMoreOlder && onLoadOlder) setPendingQuestionJump(current => current?.messageId === targetMessageId ? current : {
        conversation_id: conversationContext!.conversation_id!, messageId: targetMessageId, loadOlder: true, align: 'center', behavior: 'smooth',
      });
      return;
    }

    handledTargetKeyRef.current = targetKey;
    setHighlightedMessageId(targetMessageId);
    hideScrollButton();

    requestAnimationFrame(() => requestAnimationFrame(() => {
      const root = document.getElementById(`message-${getProcessedItemAnchorId(displayList[targetIndex])}`);
      const targetElement = Array.from(root?.querySelectorAll<HTMLElement>('[data-process-source-message-id]') ?? [])
        .find(element => element.dataset.processSourceMessageId === targetMessageId) ?? root;
      scrollElementIntoView(targetElement, {
        behavior: 'smooth',
        block: 'center',
      });
    }));

    const timer = window.setTimeout(() => {
      setHighlightedMessageId((current) => (current === targetMessageId ? undefined : current));
    }, 2400);

    return () => window.clearTimeout(timer);
  }, [displayList, hideScrollButton, location.key, scrollElementIntoView, targetMessageId, hasMoreOlder, onLoadOlder, conversationContext]);

  useEffect(() => {
    const handleMessageJump = (event: Event) => {
      const detail = (event as CustomEvent<ChatMessageJumpDetail>).detail;
      if (!detail || !detail.conversation_id) return;
      if (!conversationContext?.conversation_id || detail.conversation_id !== conversationContext.conversation_id)
        return;

      const targetIndex = displayList.findIndex((item) => {
        const sourceMessageIds = getProcessedItemSourceMessageIds(item);
        if (detail.messageId && sourceMessageIds.includes(detail.messageId)) return true;
        if (detail.msgId && sourceMessageIds.includes(detail.msgId)) return true;
        return false;
      });
      if (targetIndex < 0) {
        if (detail.loadOlder) setPendingQuestionJump(detail);
        return;
      }
      setPendingQuestionJump(null);
      setHighlightedMessageId(detail.messageId ?? detail.msgId);

      hideScrollButton();
      requestAnimationFrame(() => requestAnimationFrame(() => {
        const root = document.getElementById(`message-${getProcessedItemAnchorId(displayList[targetIndex])}`);
        const target = detail.messageId ?? detail.msgId;
        const targetElement = Array.from(root?.querySelectorAll<HTMLElement>('[data-process-source-message-id]') ?? [])
          .find(element => element.dataset.processSourceMessageId === target) ?? root;
        scrollElementIntoView(targetElement, {
          block: detail.align || 'start',
          behavior: detail.behavior || 'smooth',
        });
      }));
    };

    window.addEventListener(CHAT_MESSAGE_JUMP_EVENT, handleMessageJump);
    return () => {
      window.removeEventListener(CHAT_MESSAGE_JUMP_EVENT, handleMessageJump);
    };
  }, [conversationContext?.conversation_id, displayList, hideScrollButton, scrollElementIntoView]);

  useEffect(() => {
    if (!pendingQuestionJump) return;
    if (pendingQuestionJump.conversation_id !== conversationContext?.conversation_id) {
      setPendingQuestionJump(null);
      return;
    }
    const found = displayList.some((item) => {
      const ids = getProcessedItemSourceMessageIds(item);
      return Boolean((pendingQuestionJump.messageId && ids.includes(pendingQuestionJump.messageId))
        || (pendingQuestionJump.msgId && ids.includes(pendingQuestionJump.msgId)));
    });
    if (found) {
      window.dispatchEvent(new CustomEvent<ChatMessageJumpDetail>(CHAT_MESSAGE_JUMP_EVENT, {
        detail: { ...pendingQuestionJump, loadOlder: false },
      }));
    } else if (!loadingQuestionPage && !loadingOlder && hasMoreOlder && onLoadOlder) {
      const attemptedJump = pendingQuestionJump;
      setLoadingQuestionPage(true);
      void Promise.resolve(onLoadOlder()).then((advanced) => {
        if (advanced === false) setPendingQuestionJump(current => current === attemptedJump ? null : current);
      }).catch(() => {
        setPendingQuestionJump(current => current === attemptedJump ? null : current);
      }).finally(() => setLoadingQuestionPage(false));
    } else if (!loadingOlder && !hasMoreOlder) {
      setPendingQuestionJump(null);
    }
  }, [conversationContext?.conversation_id, displayList, hasMoreOlder, loadingOlder, loadingQuestionPage, onLoadOlder, pendingQuestionJump]);

  // Click scroll button
  const handleScrollButtonClick = () => {
    hideScrollButton();
    scrollToBottom('smooth');
  };

  const renderTurnDisclosure = (item: ITurnProcessDisclosureVO | undefined, highlighted: boolean) => {
    if (!item) return <TurnProcessDisclosure<IRenderableItem>
      preparationLabel={t('conversation.pending.preparing')}
      renderProcessItem={() => null}
      getProcessItemKey={getProcessedItemAnchorId}
      getProcessItemState={getProcessItemState}
    />;
    const linkedDelegation = delegation?.turnId === item.msg_id ? delegation : undefined;
    const presentation = delegatedTurnPresentation(item, linkedDelegation);
    type DisplayProcessItem = IRenderableItem | DelegationProcessItem;
    const getDisclosureProcessItemState = (processItem: DisplayProcessItem): TurnDisclosureProcessState =>
      processItem.type === 'delegation_record' ? processItem.records[0].kind === 'failed' ? 'failed' : 'completed' : processItem.processSegmentState ?? item.processItemStates[getProcessedItemAnchorId(processItem)] ?? getProcessItemState(processItem);
    const canonicalProcessItems = orderProcessSegments(selectJournalProcessItems(item.processItems, {
      running: item.running,
      isPrivateActivity: isPrivateJournalActivity,
      isRunning: (processItem) => getDisclosureProcessItemState(processItem) === 'running',
      textOf: (processItem) => processItem.type === 'text' ? toDisplayText(processItem.content.content) : undefined,
      finalText: item.finalAnswer,
    }));
    const turnRecords = delegationRecords.records.filter(record => (record.source.source_message_id === conversationContext?.activeRequestMessageId && item.msg_id === conversationContext?.activeTurnId)
      || delegationOwners.some(owner => owner.role === 'user' && owner.turnId === item.msg_id
        && (owner.sourceMessageIds?.includes(record.source.source_message_id) || owner.turnId === record.source.source_message_id)));
    const visibleProcessItems = insertDelegationRecords(canonicalProcessItems.flatMap((processItem): IRenderableItem[] => {
      if (processItem.type !== 'tool_summary') return [processItem];
      const messages = withoutRecordedDelegationCalls(processItem.messages, turnRecords);
      return messages.length ? [{ ...processItem, messages }] : [];
    }), turnRecords);
    const renderJournalProcessItem = (processItem: DisplayProcessItem) => {
      if (processItem.type === 'delegation_record') return <DelegationRecordRow records={processItem.records} />;
      const processState = getDisclosureProcessItemState(processItem);
      const layoutKind = getProcessItemLayoutKind(processItem);

      if (layoutKind === 'text' || layoutKind === 'thinking' || layoutKind === 'tool') {
        return renderProcessTraceItem(
          processItem,
          workspaceRoots,
          layoutKind === 'thinking' || !item.running ? processState : undefined
        );
      }

      const summary = buildProcessReceiptSummary(
        processItem,
        processState,
        t,
        workspaceRoots,
        { language: toolLanguage }
      );
      const attention = summary.attention === true;
      return (
        <TurnProcessReceipt
          receipt={{
            id: `journal-receipt-${getProcessedItemAnchorId(processItem)}`,
            item: processItem,
            label: summary.label,
            state: processState,
            icon: summary.icon,
            defaultExpanded: summary.defaultExpanded,
            hasDetail: summary.hasDetail,
            attention,
          }}
          highlighted={highlighted}
          renderProcessItem={(detailItem) =>
            renderProcessTraceItem(
              detailItem,
              workspaceRoots,
              item.running ? undefined : processState
            )
          }
        />
      );
    };

    return (
      <TurnProcessDisclosure
        item={{ ...presentation, processItems: visibleProcessItems }}
        highlighted={highlighted}
        renderProcessItem={renderJournalProcessItem}
        getProcessItemKey={(entry) => entry.type === 'delegation_record' ? entry.id : getProcessedItemAnchorId(entry)}
        getProcessItemState={getDisclosureProcessItemState}
        getProcessItemLayoutKind={(entry) => entry.type === 'delegation_record' ? 'delegation' : getProcessItemLayoutKind(entry)}
        getProcessGroupId={(processItem) => processItem.processGroupId}
        getProcessItemSourceId={(processItem) => processItem.type === 'delegation_record' ? processItem.records[0].source.source_message_id : getProcessedItemSourceMessageIds(processItem)[0]}
        isProcessItemHighlighted={(processItem) => Boolean(highlightedMessageId && (processItem.type === 'delegation_record' ? processItem.records[0].source.source_message_id === highlightedMessageId : getProcessedItemSourceMessageIds(processItem).includes(highlightedMessageId)))}

      />
    );
  };

  const renderProcessReceipt = (item: IProcessReceiptVO, highlighted: boolean) => {
    return (
      <TurnProcessReceipt
        receipt={item}
        highlighted={highlighted}
        renderProcessItem={(processItem) =>
          renderProcessTraceItem(processItem, workspaceRoots)
        }
      />
    );
  };

  const renderItem = (_index: number, item: (typeof displayList)[0], renderKey = item.id) => {
    const highlighted = matchesTargetMessage(item, highlightedMessageId);
    const compactError = item.type === 'tips' && shouldCompactPermissionBlockedSummary(
      permissionMessageSources.find(message => message.id === item.id) ?? item, permissionNotices);
    if ('type' in item && item.type === 'turn_process_disclosure') {
      return (
        <div
          key={renderKey}
          id={`message-${getProcessedItemAnchorId(item)}`}
          data-testid='turn-process-disclosure'
          className='min-w-0 message-item m-t-10px turn_process_disclosure'
          style={highlighted ? highlightStyle : undefined}
        >
          {renderTurnDisclosure(item, highlighted)}
        </div>
      );
    }
    if ('type' in item && item.type === 'process_receipt') {
      return (
        <div
          key={item.id}
          id={`message-${getProcessedItemAnchorId(item)}`}
          data-testid='turn-process-receipt'
          className='min-w-0 message-item m-t-10px process_receipt'
          style={highlighted ? highlightStyle : undefined}
        >
          {renderProcessReceipt(item, highlighted)}
        </div>
      );
    }
    if ('type' in item && item.type === 'turn_deliverables') {
      return (
        <div
          key={item.id}
          id={`message-${getProcessedItemAnchorId(item)}`}
          data-testid='turn-deliverables'
          className='min-w-0 message-item m-t-10px turn_deliverables'
          style={highlighted ? highlightStyle : undefined}
        >
          <TurnDeliverablesCard
            items={item.items}
            workspace={conversationContext?.workspace}
            partial={item.partial}
          />
        </div>
      );
    }
    if ('type' in item && item.type === 'turn_creation_tasks') {
      return <ConversationCreationTaskCards messageId={item.message_id} />;
    }
    if ('type' in item && item.type === 'turn_knowledge_writeback') {
      return <div key={item.id} id={`message-${item.id}`} className='min-w-0 message-item turn_knowledge_writeback'>
        <KnowledgeWritebackNotice notice={item.notice} />
      </div>;
    }
    if ('type' in item && item.type === 'turn_permission_required') {
      return <div key={item.id} id={`message-${item.id}`} className='min-w-0 message-item turn_permission_required'
        style={highlighted ? highlightStyle : undefined}>
        <SystemPermissionNotice notice={item} />
      </div>;
    }
    if ('type' in item && item.type === 'turn_actions') {
      return (
        <div
          key={item.id}
          id={`message-${getProcessedItemAnchorId(item)}`}
          data-testid='turn-actions'
          className='min-w-0 message-item turn_actions'
          style={highlighted ? highlightStyle : undefined}
        >
          <MessageText message={item.message} actionsOnly />
        </div>
      );
    }
    const continuation = item.type === 'text' ? item.content.continuation_of_message_id : undefined;
    const sourceLoaded = !continuation || processedList.some(source =>
      source.type === 'text' && source.position === 'left' && source.turn_id === item.turn_id &&
      getProcessedItemSourceMessageIds(source).includes(continuation));
    return (
      <React.Fragment key={renderKey}>
      {!sourceLoaded && <div role='status' data-testid='missing-reply-continuation' className='text-t-tertiary text-sm m-t-10px'>
        {t('messages.missingReplyContinuation')}
        {hasMoreOlder && onLoadOlder && <button type='button' disabled={loadingOlder} onClick={() => void onLoadOlder()}>
          {t('messages.loadEarlierReply')}
        </button>}
      </div>}
      <MessageItem
        message={item as TMessage}
        key={renderKey}
        highlighted={highlighted}
        compactError={compactError}
        hideActions={
          isActiveProcessTextItem(item, _index) ||
          movedActionMessageIds.has((item as TMessage).id)
        }
      ></MessageItem>
      </React.Fragment>
    );
  };

  // These are React keys only. Runtime state and timing continue to come from
  // the ordinary timeline, including a later canonical root replacing its
  // accepted-user boundary. Keep the first request/status DOM nodes mounted.
  const initialRenderKeys = useRef<{ conversationId?: ConversationId; renderKey?: string; requestMessageId?: MessageId }>({});
  const savedKeys = initialRenderKeys.current.conversationId === conversationContext?.conversation_id ? initialRenderKeys.current : undefined;
  const hasPendingPresentation = Boolean(pendingMessage && pendingMessage.conversation_id === conversationContext?.conversation_id);
  const initialRenderKey = hasPendingPresentation ? pendingMessage!.id : savedKeys?.renderKey;
  const initialRequestIndex = displayList.findIndex(item => item.type === 'text' && item.position === 'right'
    && item.conversation_id === conversationContext?.conversation_id && (savedKeys?.requestMessageId
      ? getMessageBusinessIdentity(item) === savedKeys.requestMessageId : hasPendingPresentation));
  const initialRequest = initialRequestIndex >= 0 ? displayList[initialRequestIndex] : undefined;
  const initialRequestMessageId = initialRequest?.type === 'text' ? getMessageBusinessIdentity(initialRequest) : undefined;
  const initialProgressIndex = initialRequestIndex >= 0 && displayList[initialRequestIndex + 1]?.type === 'turn_process_disclosure'
    ? initialRequestIndex + 1 : hasPendingPresentation && initialRequestIndex < 0
      ? displayList.findIndex(item => item.type === 'turn_process_disclosure') : -1;
  const hasInitialResult = Boolean(pauseError || (initialRequestIndex >= 0 && list.some(message =>
    message.conversation_id === conversationContext?.conversation_id && (isTerminalAssistantItem(message)
      || getProcessedItemTurnEndedAt(message) !== undefined))));
  const showPendingMessage = hasPendingPresentation && initialRequestIndex < 0 && !hasInitialResult;
  const showPreparation = hasPendingPresentation && initialProgressIndex < 0 && !hasInitialResult;
  useLayoutEffect(() => {
    if (hasPendingPresentation) {
      initialRenderKeys.current = { conversationId: conversationContext?.conversation_id,
        renderKey: pendingMessage!.id, requestMessageId: initialRequestMessageId };
      // A user-created notification can precede the send receipt/Turn state.
      // Only the committed replacement progress or result ends preparation.
      if (initialRequestMessageId && (initialProgressIndex >= 0 || hasInitialResult)) onPendingMessageReady?.();
    } else if (initialRenderKeys.current.conversationId !== conversationContext?.conversation_id) {
      initialRenderKeys.current = {};
    }
  }, [hasPendingPresentation, pendingMessage, initialRequestMessageId, initialProgressIndex, hasInitialResult, conversationContext?.conversation_id, onPendingMessageReady]);

  const preparationRow = showPreparation ? <React.Fragment key={`${initialRenderKey}-progress`}>
    <div key={`${initialRenderKey}-progress`} className='min-w-0 message-item m-t-10px turn_process_disclosure' data-testid='initial-message-preparing'>
      {renderTurnDisclosure(undefined, false)}
    </div>
  </React.Fragment> : null;
  const renderedRows: React.ReactNode[] = [];
  if (showPendingMessage && pendingMessage) renderedRows.push(<React.Fragment key={`${initialRenderKey}-request`}>
    {renderItem(-1, pendingMessage, `${initialRenderKey}-request`)}
  </React.Fragment>);
  if (showPreparation && initialRequestIndex < 0) renderedRows.push(preparationRow);
  displayList.forEach((item, index) => {
    const key = initialRenderKey && index === initialProgressIndex ? `${initialRenderKey}-progress`
      : initialRenderKey && index === initialRequestIndex ? `${initialRenderKey}-request` : item.id;
    renderedRows.push(<React.Fragment key={key}>{renderItem(index, item, key)}</React.Fragment>);
    if (showPreparation && index === initialRequestIndex) renderedRows.push(preparationRow);
  });

  if (displayList.length === 0 && isMessageListLoading && !showPendingMessage && !showPreparation) {
    return <MessageListSkeleton />;
  }

  if (displayList.length === 0 && !showPendingMessage && !showPreparation && !pauseError && emptySlot && conversationContext?.isProcessing !== true) {
    return <div className={`relative flex-1 h-full flex items-center justify-center ${contentStyles.emptyState}`}>{emptySlot}</div>;
  }

  return (
    <ToolDetailsProvider scope={conversationContext?.conversation_id}>
    <ConversationProseContext.Provider value={prosePresentation}>
    <div className='relative flex-1 h-full'>
      <ConversationQuestionLocator conversation_id={conversationContext?.conversation_id} />

      {/* Use PreviewGroup to wrap all messages for cross-message image preview */}
      <Image.PreviewGroup
        className='conversation-image-preview'
        actionsLayout={['zoomIn', 'zoomOut', 'originalSize', 'rotateLeft', 'rotateRight']}
      >
        <ImagePreviewContext.Provider value={{ inPreviewGroup: true }}>
          <NomiScrollArea
            ref={handleScrollerRef}
            data-testid='message-list-scroller'
            className={`flex-1 h-full pb-10px box-border ${contentStyles.scroller}`}
            style={{ overflowAnchor: 'none' }}
            onPointerDown={handlePointerDown}
            onKeyDown={handleKeyDown}
            onScroll={handleScrollWithPaging}
            onWheel={handleWheel}
          >
            <div ref={handleColumnRef} className={contentStyles.column} data-testid='message-list-content' style={{ overflowAnchor: 'none' }}>
              <div className='h-10px' />
              {renderedRows}
              {delegationRecords.hasMore && <button type='button' className='turn-process-records-more' disabled={delegationRecords.loading} onClick={delegationRecords.loadMore}>{t('messages.delegation.more')}</button>}
              {delegationRecords.failed && <p role='alert' className='turn-process-trace-detail__notice'>{t('messages.delegation.loadFailed')} <button type='button' onClick={delegationRecords.retry}>{t('messages.delegation.retry')}</button></p>}
              {pauseError && <div className='mt-6px' data-testid='conversation-pause-error'><MessageTips message={pauseError} /></div>}
              {conversationContext?.executionWait && <div role='note' data-testid='conversation-waiting-notice'
                className='mt-6px bg-message-tips rd-8px px-12px py-10px text-t-secondary text-13px'>
                {t(`messages.waitingReasons.${conversationContext.executionWait.outcome.reason}`)}
              </div>}
              <div className='h-20px' />
            </div>
          </NomiScrollArea>
        </ImagePreviewContext.Provider>
      </Image.PreviewGroup>

      {showScrollButton && (
        <>
          {/* Gradient mask */}
          <div className='absolute bottom-0 left-0 right-0 h-100px pointer-events-none' />
          {/* Scroll button */}
          <div className='absolute bottom-20px left-1/2 -translate-x-1/2 z-100'>
            <button
              type='button'
              className='flex items-center justify-center w-32px h-32px p-0 rd-full bg-base shadow-lg cursor-pointer hover:bg-1 transition-colors border-1px border-solid border-3'
              onClick={handleScrollButtonClick}
              title={t('messages.scrollToBottom')}
              aria-label={t('messages.scrollToBottom')}
              style={{ lineHeight: 0 }}
            >
              <Down theme='filled' size='14' fill={iconColors.primary} strokeWidth={5} style={{ display: 'block' }} />
            </button>
          </div>
        </>
      )}

      <SelectionReplyButton messages={list} />
    </div>
    </ConversationProseContext.Provider>
    </ToolDetailsProvider>
  );
};

export default MessageList;
