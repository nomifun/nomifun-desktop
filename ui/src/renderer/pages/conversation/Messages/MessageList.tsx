/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IConversationArtifact } from '@/common/adapter/ipcBridge';
import type {
  IMessageText,
  IMessageToolCall,
  IMessageToolGroup,
  TMessage,
} from '@/common/chat/chatLib';
import { normalizeToolMessages } from '@/common/chat/normalizeToolCall';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { iconColors } from '@/renderer/styles/colors';
import { CHAT_MESSAGE_JUMP_EVENT, type ChatMessageJumpDetail } from '@/renderer/utils/chat/chatMinimapEvents';
import { Image } from '@arco-design/web-react';
import { Down } from '@icon-park/react';
import MessagePermission from './components/MessagePermission';
import classNames from 'classnames';
import React, { createContext, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation } from 'react-router-dom';
import { uuid } from '@renderer/utils/common';
import './messages.css';
import HOC from '@renderer/utils/ui/HOC';
import type { FileChangeInfo } from './MessageFileChanges';
import { parseDiff } from './MessageFileChanges';
import { useConversationArtifacts } from './artifacts';
import { useKnowledgeWritebackEvents, useMessageList, useMessageListLoading } from './hooks';
import MessageAgentStatus from './components/MessageAgentStatus';
import MessageTips from './components/MessageTips';
import MessageToolCall from './components/MessageToolCall';
import MessageToolGroup from './components/MessageToolGroup';
import { isSuccessfulWriteFileResult } from './components/toolGroupArtifactVisibility';
import MessageCronTrigger from './components/MessageCronTrigger';
import MessageSkillSuggest from './components/MessageSkillSuggest';
import MessageText from './components/MessageText';
import MessageThinking from './components/MessageThinking';
import MessageListSkeleton from './components/MessageListSkeleton';
import TurnProcessDisclosure from './components/TurnProcessDisclosure';
import TurnProcessReceipt, { type TurnProcessReceiptIcon } from './components/TurnProcessReceipt';
import {
  buildToolReceiptSummaryParts,
  buildToolSummaryDescriptor,
  getToolReceiptIconFromSummaryParts,
  type ToolReceiptSummaryPart,
} from './components/toolGroupSummaryModel';
import ProcessTraceItem, { type ProcessTraceItemExpansionControls } from './components/ProcessTraceItem';
import { isContextCompressionTip } from './processTipModel';
import { formatFileTargetPreview, splitToolReceiptTargets } from './processFileTargetLabel';
import type { WriteFileResult } from './types';
import { useAutoScroll } from './useAutoScroll';
import { useAutoPreviewOfficeFiles } from '@/renderer/hooks/file/useAutoPreviewOfficeFiles';
import { useAutoPreviewMiniApp } from '@/renderer/hooks/file/useAutoPreviewMiniApp';
import SelectionReplyButton from './components/SelectionReplyButton';
import ConversationQuestionLocator from '../components/ConversationTitleMinimap/ConversationQuestionLocator';
import {
  assignTurnIdsFromUserRequests,
  buildTurnDisclosureItems,
  type TurnDisclosureProcessState,
  type TurnDisclosureInputItem,
  type TurnDisclosureOutputItem,
} from './turnDisclosureModel';
import { getProcessItemState } from './turnProcessState';
import { planTurnLiveStep } from './turnLiveStepModel';
import {
  collectTurnDeliverables,
  type TurnDeliverableCandidate,
  type TurnDeliverableItem,
  type TurnGateInfo,
} from './turnDeliverablesModel';
import TurnDeliverablesCard from './components/TurnDeliverablesCard';
import { isSupersededPlanToolFailure } from './planToolVisibility';
import type { MessageId } from '@/common/types/ids';
import { ExplicitToolRetryReceiptIndex } from './toolRetryReceiptModel';

type SourceMessageId = MessageId;

type IMessageVO =
  | TMessage
  | {
      type: 'file_summary';
      id: string;
      msg_id?: MessageId;
      turn_id?: MessageId;
      diffs: FileChangeInfo[];
      sourceMessageIds: SourceMessageId[];
      created_at: number;
    }
  | {
      type: 'tool_summary';
      id: string;
      msg_id?: MessageId;
      turn_id?: MessageId;
      messages: Array<IMessageToolGroup | IMessageToolCall>;
      sourceMessageIds: SourceMessageId[];
      created_at: number;
    };
type ToolSummaryVO = Extract<IMessageVO, { type: 'tool_summary' }>;
type IArtifactVO = { type: 'artifact'; id: string; artifact: IConversationArtifact; created_at: number };
type IRenderableItem = IMessageVO | IArtifactVO;
type ITurnProcessDisclosureVO = {
  type: 'turn_process_disclosure';
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
};
type ITurnDeliverablesVO = {
  type: 'turn_deliverables';
  id: string;
  turn_id: MessageId;
  items: TurnDeliverableItem[];
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
type ITurnLiveStepVO = {
  type: 'turn_live_step';
  id: string;
  msg_id: MessageId;
  label: string;
  state: 'running' | 'waiting';
  icon: TurnProcessReceiptIcon;
  sourceMessageIds: SourceMessageId[];
  created_at: number;
};
type IProcessedItem =
  | IRenderableItem
  | ITurnProcessDisclosureVO
  | IProcessReceiptVO
  | ITurnDeliverablesVO
  | ITurnActionsVO
  | ITurnLiveStepVO;

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
      item.type === 'turn_live_step')
  ) {
    return item.sourceMessageIds;
  }
  if ('type' in item && item.type === 'artifact') return [];
  if ('type' in item && item.type === 'tool_summary') {
    return item.sourceMessageIds;
  }
  if ('type' in item && item.type === 'file_summary') {
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
      'file_summary',
      'tool_summary',
      'artifact',
      'turn_process_disclosure',
      'process_receipt',
      'turn_deliverables',
      'turn_actions',
      'turn_live_step',
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

const getProcessedItemProcessStartedAt = (item: IRenderableItem): number => getProcessedItemCreatedAt(item);

const getProcessedItemProcessEndedAt = (item: IRenderableItem): number => {
  const createdAt = getProcessedItemCreatedAt(item);
  const duration = getThinkingDurationMs(item);
  if (duration === undefined) return createdAt;
  return createdAt + duration;
};

const getProcessedItemMsgId = (item: IRenderableItem): MessageId | undefined => {
  if ('type' in item && (item.type === 'file_summary' || item.type === 'tool_summary')) {
    return item.msg_id;
  }
  if ('type' in item && item.type === 'artifact') {
    return undefined;
  }
  return item.msg_id;
};

const getProcessedItemTurnId = (item: IRenderableItem): MessageId | undefined => {
  if ('type' in item && item.type === 'artifact') return undefined;
  return item.turn_id;
};

const getProcessedItemRole = (item: IRenderableItem): TurnDisclosureInputItem['role'] => {
  if ('type' in item && (item.type === 'file_summary' || item.type === 'tool_summary')) {
    return 'process';
  }
  if ('type' in item && item.type === 'artifact') {
    return 'other';
  }

  switch (item.type) {
    case 'text':
      return item.position === 'right' ? 'user' : 'assistant';
    case 'tips':
      if (isContextCompressionTip(item)) return 'process';
      return 'assistant';
    case 'thinking':
      return 'process_content';
    case 'tool_call':
    case 'tool_group':
    case 'agent_status':
    case 'permission':
      return 'process';
    default:
      return 'other';
  }
};

type TranslationFn = ReturnType<typeof useTranslation>['t'];

const defaultToolSummaryByState: Record<TurnDisclosureProcessState, string> = {
  completed: 'Ran {{target}}',
  running: 'Running {{target}}',
  waiting: 'Waiting to confirm {{target}}',
  failed: 'Failed {{target}}',
  canceled: 'Canceled {{target}}',
};

const compactReceiptText = (value: unknown, fallback: string): string => {
  if (typeof value !== 'string') return fallback;
  const compacted = value.replace(/\s+/g, ' ').trim();
  return compacted || fallback;
};

const getToolReceiptDisplayTarget = (part: ToolReceiptSummaryPart, workspaceRoots: string[]): string | undefined => {
  if (!part.target) return undefined;
  if (part.action !== 'read_files' && part.action !== 'edit_files') return part.target;
  const targets = splitToolReceiptTargets(part.target);
  return targets.length ? formatFileTargetPreview(targets, { workspaceRoots }) : part.target;
};

const formatToolReceiptPart = (
  part: ToolReceiptSummaryPart,
  t: TranslationFn,
  workspaceRoots: string[]
): string => {
  const displayTarget = getToolReceiptDisplayTarget(part, workspaceRoots);

  if (part.skipped) {
    return t('messages.toolSummary.skipped', {
      target:
        displayTarget ??
        t('messages.processReceipt.tools', {
          count: part.count,
          defaultValue: '{{count}} tools',
        }),
      defaultValue: 'Skipped {{target}}',
    });
  }

  if (part.notExecutedReason === 'invalid_arguments') {
    return t('messages.toolSummary.invalidArguments', {
      target: displayTarget ?? t('messages.processReceipt.tool', { defaultValue: 'tool' }),
      defaultValue: 'Arguments did not pass validation; {{target}} was not run',
    });
  }

  if ((part.state === 'failed' || part.state === 'canceled') && displayTarget) {
    return t(`messages.toolSummary.${part.state}`, {
      target: displayTarget,
      defaultValue: defaultToolSummaryByState[part.state],
    });
  }

  switch (part.action) {
    case 'read_files':
      if (displayTarget) {
        return part.state === 'running'
          ? t('messages.processReceipt.readingTargets', {
              count: part.count,
              target: displayTarget,
              defaultValue: 'Reading {{count}} files: {{target}}',
            })
          : t('messages.processReceipt.readTargets', {
              count: part.count,
              target: displayTarget,
              defaultValue: 'Read {{count}} files: {{target}}',
            });
      }
      return part.state === 'running'
        ? t('messages.processReceipt.readingFiles', {
            count: part.count,
            defaultValue: 'Reading {{count}} files',
          })
        : t('messages.processReceipt.readFiles', {
            count: part.count,
            defaultValue: 'Read {{count}} files',
          });
    case 'edit_files':
      if (displayTarget) {
        return part.state === 'running'
          ? t('messages.processReceipt.editingFileTargets', {
              count: part.count,
              target: displayTarget,
              defaultValue: 'Editing {{count}} files: {{target}}',
            })
          : t('messages.processReceipt.fileEditTargets', {
              count: part.count,
              target: displayTarget,
              defaultValue: 'Edited {{count}} files: {{target}}',
            });
      }
      return part.state === 'running'
        ? t('messages.processReceipt.editingFiles', {
            count: part.count,
            defaultValue: 'Editing {{count}} files',
          })
        : t('messages.processReceipt.fileEdits', {
            count: part.count,
            defaultValue: 'Edited {{count}} files',
          });
    case 'run_commands':
      if (part.count === 1 && part.target) {
        return t(`messages.toolSummary.${part.state}`, {
          target: part.target,
          defaultValue: defaultToolSummaryByState[part.state],
        });
      }
      return part.state === 'running'
        ? t('messages.processReceipt.runningCommands', {
            count: part.count,
            defaultValue: 'Running {{count}} commands',
          })
        : t('messages.processReceipt.runCommands', {
            count: part.count,
            defaultValue: 'Ran {{count}} commands',
          });
    case 'search_code':
      return part.state === 'running'
        ? t('messages.processReceipt.searchingCode', { defaultValue: 'Searching code' })
        : t('messages.processReceipt.searchedCode', { defaultValue: 'Searched code' });
    case 'list_files':
      return part.state === 'running'
        ? t('messages.processReceipt.listingFiles', { defaultValue: 'Listing files' })
        : t('messages.processReceipt.listedFiles', { defaultValue: 'Listed files' });
    case 'load_tools':
      return part.state === 'running'
        ? t('messages.processReceipt.loadingTools', {
            count: part.count,
            defaultValue: 'Loading {{count}} tools',
          })
        : t('messages.processReceipt.loadedTools', {
            count: part.count,
            defaultValue: 'Loaded {{count}} tools',
          });
    case 'generic':
    default:
      if (displayTarget) {
        return t(`messages.toolSummary.${part.state}`, {
          target: displayTarget,
          defaultValue: defaultToolSummaryByState[part.state],
        });
      }
      return t('messages.processReceipt.tools', {
        count: part.count,
        defaultValue: '{{count}} tools',
      });
  }
};

const getToolReceiptIcon = (
  messages: Array<IMessageToolGroup | IMessageToolCall>
): TurnProcessReceiptIcon => {
  const latestMessage = messages.findLast(Boolean);
  if (!latestMessage) return 'tool';

  if (latestMessage.type === 'tool_group') {
    if (!Array.isArray(latestMessage.content)) return 'tool';
    const latestTool = latestMessage.content.findLast(Boolean);
    const confirmationType = latestTool?.confirmationDetails?.type;
    if (confirmationType === 'edit') return 'edit';
    if (confirmationType === 'info') return 'file';
    return 'tool';
  }

  const toolName = `${latestMessage.content.name ?? ''} ${latestMessage.content.description ?? ''}`.toLowerCase();
  if (/\b(write|edit|patch|update|modify)\b/.test(toolName)) return 'edit';
  if (/\b(read|list|ls|glob|search|grep|find)\b/.test(toolName)) return 'file';
  return 'tool';
};

const buildProcessReceiptSummary = (
  item: IRenderableItem,
  state: TurnDisclosureProcessState,
  t: TranslationFn,
  workspaceRoots: string[] = []
): { label: string; icon: TurnProcessReceiptIcon; defaultExpanded: boolean; hasDetail?: boolean } => {
  if ('type' in item && item.type === 'tool_summary') {
    const tools = normalizeToolMessages(item.messages);
    const receiptParts = buildToolReceiptSummaryParts(tools, state);
    const descriptor = buildToolSummaryDescriptor(tools, state);
    const label = receiptParts.length
      ? receiptParts.map((part) => formatToolReceiptPart(part, t, workspaceRoots)).join(' ')
      : descriptor
        ? t(`messages.toolSummary.${state}`, {
            target: descriptor.target,
            defaultValue: defaultToolSummaryByState[state],
          })
        : t('messages.processReceipt.tools', {
            count: item.messages.length,
            defaultValue: '{{count}} tools',
          });
    return {
      label,
      icon: getToolReceiptIconFromSummaryParts(receiptParts) ?? getToolReceiptIcon(item.messages),
      defaultExpanded: state === 'waiting',
      hasDetail: true,
    };
  }

  if ('type' in item && item.type === 'file_summary') {
    const targets = item.diffs
      .map((file) => file.fullPath || file.file_name)
      .filter((target): target is string => Boolean(target));
    const targetPreview = targets.length ? formatFileTargetPreview(targets, { workspaceRoots }) : '';
    return {
      label: targetPreview
        ? t('messages.processReceipt.fileEditTargets', {
            count: item.diffs.length,
            target: targetPreview,
            defaultValue: 'Edited {{count}} files: {{target}}',
          })
        : t('messages.processReceipt.fileEdits', {
            count: item.diffs.length,
            defaultValue: 'Edited {{count}} files',
          }),
      icon: 'edit',
      defaultExpanded: false,
      hasDetail: item.diffs.length > 1,
    };
  }

  if ('type' in item && item.type === 'artifact') {
    const target =
      item.artifact.kind === 'cron_trigger' ? item.artifact.payload.cron_job_name : item.artifact.payload.name;
    return {
      label: t('messages.processReceipt.status', { target, defaultValue: '{{target}}' }),
      icon: 'status',
      defaultExpanded: false,
      hasDetail: false,
    };
  }

  switch (item.type) {
    case 'permission':
      return {
        label: t('messages.processReceipt.waitingPermission', {
          target: compactReceiptText(item.content.title || item.content.description, t('messages.permissionRequest')),
          defaultValue: 'Waiting to confirm {{target}}',
        }),
        icon: 'permission',
        defaultExpanded: true,
        hasDetail: true,
      };
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
        icon: state === 'failed' ? 'permission' : 'status',
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
        workspaceRoots
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
  variant: 'list' | 'receipt' = 'list',
  workspaceRoots: string[] = [],
  stateOverride?: TurnDisclosureProcessState,
  thinkingExpansion?: ProcessTraceItemExpansionControls
) => (
  <ProcessTraceItem
    item={item}
    variant={variant}
    workspaceRoots={workspaceRoots}
    stateOverride={stateOverride}
    thinkingExpansion={thinkingExpansion}
  />
);

const isCompletedThinkingProcessItem = (item: IRenderableItem): boolean =>
  'type' in item && item.type === 'thinking' && item.content.status === 'done';

const getProcessItemLayoutKind = (item: IRenderableItem): string => {
  if ('type' in item && item.type === 'text') return 'text';
  if ('type' in item && item.type === 'thinking') return 'thinking';
  if (
    'type' in item &&
    ['tool_summary', 'file_summary', 'tool_call', 'tool_group'].includes(item.type)
  ) {
    return 'tool';
  }
  if ('type' in item && item.type === 'permission') return 'permission';
  if ('type' in item && (item.type === 'agent_status' || item.type === 'tips' || item.type === 'artifact')) return 'status';
  return 'other';
};

const MessageItem: React.FC<{ message: TMessage; highlighted?: boolean; hideActions?: boolean }> = React.memo(
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
          'min-w-0 flex items-start message-item [&>div]:max-w-full px-8px m-t-10px max-w-full md:max-w-780px mx-auto',
          message.type,
          {
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
  })(({ message, hideActions }) => {
    const { t } = useTranslation();
    switch (message.type) {
      case 'text':
        return <MessageText message={message} hideActions={hideActions}></MessageText>;
      case 'tips':
        return <MessageTips message={message}></MessageTips>;
      case 'tool_call':
        return <MessageToolCall message={message}></MessageToolCall>;
      case 'tool_group':
        return <MessageToolGroup message={message}></MessageToolGroup>;
      case 'agent_status':
        return <MessageAgentStatus message={message}></MessageAgentStatus>;
      case 'permission':
        return <MessagePermission message={message}></MessagePermission>;
      case 'plan':
        // Plans render in the docked PinnedPlan bar, not inline — they're
        // filtered out of processedList above. This guard keeps the switch
        // exhaustive (the `never` default below would otherwise error).
        return null;
      case 'thinking':
        return <MessageThinking message={message}></MessageThinking>;
      case 'available_commands':
        return null;
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
    prev.hideActions === next.hideActions
);

const MessageList: React.FC<{
  className?: string;
  emptySlot?: React.ReactNode;
  /** Windowed-history paging (nomi surfaces): prepend the next older message
   *  window when the user scrolls to the top. Omitted on chats that still load
   *  their whole transcript at once. */
  onLoadOlder?: () => void | Promise<void>;
  hasMoreOlder?: boolean;
  loadingOlder?: boolean;
}> = ({ emptySlot, onLoadOlder, hasMoreOlder, loadingOlder }) => {
  const list = useMessageList();
  const isMessageListLoading = useMessageListLoading();
  const artifacts = useConversationArtifacts();
  const conversationContext = useConversationContextSafe();
  useKnowledgeWritebackEvents(conversationContext?.conversation_id);
  useAutoPreviewOfficeFiles(conversationContext);
  useAutoPreviewMiniApp(conversationContext);
  const workspaceRoots = useMemo(
    () => (conversationContext?.workspace ? [conversationContext.workspace] : []),
    [conversationContext?.workspace]
  );
  const { t } = useTranslation();
  const location = useLocation();
  const locationState = (location.state || {}) as ConversationLocationState;
  const targetMessageId = locationState.targetMessageId;
  const [highlightedMessageId, setHighlightedMessageId] = useState<MessageId | undefined>();
  const handledTargetKeyRef = useRef<string>('');

  // Pre-process message list to group tool outputs into summary cards
  const processedList = useMemo(() => {
    const result: Array<IMessageVO> = [];
    let diffsChanges: FileChangeInfo[] = [];
    let diffsSourceMessageIds: SourceMessageId[] = [];
    let diffsTurnId: MessageId | undefined;
    let toolList: Array<IMessageToolGroup | IMessageToolCall> = [];
    let toolSourceMessageIds: SourceMessageId[] = [];
    const retrySummaries = new ExplicitToolRetryReceiptIndex<ToolSummaryVO>();

    const pushFileDffChanges = (
      changes: FileChangeInfo,
      sourceMessageId: SourceMessageId,
      created_at: number,
      msg_id?: MessageId,
      turn_id?: MessageId
    ) => {
      if (diffsChanges.length && diffsTurnId && turn_id && diffsTurnId !== turn_id) {
        diffsChanges = [];
        diffsSourceMessageIds = [];
      }
      if (!diffsChanges.length) {
        diffsSourceMessageIds = [];
        diffsTurnId = turn_id;
        result.push({
          type: 'file_summary',
          id: `summary-${sourceMessageId}`,
          msg_id,
          turn_id,
          diffs: diffsChanges,
          sourceMessageIds: diffsSourceMessageIds,
          created_at,
        });
      }
      diffsChanges.push(changes);
      diffsSourceMessageIds.push(sourceMessageId);
      toolList = [];
      toolSourceMessageIds = [];
    };
    const pushToolList = (message: IMessageToolGroup | IMessageToolCall) => {
      const existingRetry = message.type === 'tool_call' ? retrySummaries.takeContinuation(message) : undefined;
      if (message.type === 'tool_call' && existingRetry) {
        existingRetry.messages.push(message);
        const sourceMessageId = getMessageBusinessIdentity(message);
        if (sourceMessageId) existingRetry.sourceMessageIds.push(sourceMessageId);
        // A retry can be separated from its first attempt by thinking/text.
        // Keep the durable summary reference above, but do not accidentally
        // append an unrelated following tool to that earlier receipt.
        toolList = [];
        toolSourceMessageIds = [];
        diffsChanges = [];
        diffsSourceMessageIds = [];
        diffsTurnId = undefined;
        return;
      }
      const groupedTurnId = toolList.find((tool) => tool.turn_id)?.turn_id;
      if (groupedTurnId && message.turn_id && groupedTurnId !== message.turn_id) {
        // A delayed event from another explicit turn must start a new receipt;
        // otherwise the synthetic summary would inherit the first tool's turn
        // and visually attach the delayed failure to the wrong request.
        toolList = [];
        toolSourceMessageIds = [];
      }
      if (!toolList.length) {
        toolSourceMessageIds = [];
        const summary: ToolSummaryVO = {
          type: 'tool_summary',
          id: `tool-summary-${message.id}`,
          msg_id: message.msg_id,
          turn_id: message.turn_id,
          messages: toolList,
          sourceMessageIds: toolSourceMessageIds,
          created_at: message.created_at ?? 0,
        };
        result.push(summary);
      }
      toolList.push(message);
      const sourceMessageId = getMessageBusinessIdentity(message);
      if (sourceMessageId) toolSourceMessageIds.push(sourceMessageId);
      if (message.type === 'tool_call') {
        const summary = result.findLast(
          (item): item is ToolSummaryVO => item.type === 'tool_summary' && item.messages === toolList
        );
        if (summary) {
          retrySummaries.rememberFirst(message, summary);
        }
      }
      diffsChanges = [];
      diffsSourceMessageIds = [];
      diffsTurnId = undefined;
    };

    for (let i = 0, len = list.length; i < len; i++) {
      const message = list[i];
      // Skip hidden and available_commands messages
      if (message.hidden) continue;
      if (
        message.type === 'tool_call' &&
        message.content.name === 'update_plan' &&
        isSupersededPlanToolFailure(message, list.slice(i + 1))
      ) {
        continue;
      }
      if (message.type === 'available_commands') continue;
      // Plans are no longer rendered inline — they surface in the docked
      // PinnedPlan bar above the composer, which reads the raw list directly.
      // A plan also closes the preceding tool receipt. Without this boundary,
      // update_plan and the next unrelated file operation are merged and a
      // failure can be labelled with the later operation's target.
      if (message.type === 'plan') {
        toolList = [];
        toolSourceMessageIds = [];
        diffsChanges = [];
        diffsSourceMessageIds = [];
        diffsTurnId = undefined;
        continue;
      }
      // Connection-handshake status banners (connecting/connected/authenticated/
      // session_active) are implementation noise: never render them as chat
      // items, and never let them fragment the tool-execution trace below.
      // Actionable 'error' status still surfaces. (Phase 3 UX)
      if (message.type === 'agent_status') {
        const st = (message.content as { status?: string })?.status;
        if (st === 'connecting' || st === 'connected' || st === 'authenticated' || st === 'session_active') {
          continue;
        }
      }
      if (message.type === 'tool_group') {
        if (message.content.length === 1) {
          const writeFileResults = message.content
            .filter(isSuccessfulWriteFileResult)
            .map((item) => item.result_display as WriteFileResult);
          const sourceMessageId = getMessageBusinessIdentity(message);
          if (writeFileResults.length && writeFileResults[0].file_diff && sourceMessageId) {
            pushFileDffChanges(
              parseDiff(writeFileResults[0].file_diff, writeFileResults[0].file_name),
              sourceMessageId,
              message.created_at ?? 0,
              message.msg_id,
              message.turn_id
            );
            continue;
          }
        }
        pushToolList(message);
        continue;
      }
      if (message.type === 'tool_call') {
        pushToolList(message);
        continue;
      }
      toolList = [];
      toolSourceMessageIds = [];
      diffsChanges = [];
      diffsSourceMessageIds = [];
      diffsTurnId = undefined;
      result.push(message);
    }
    const visibleArtifacts = artifacts
      .filter((artifact) => {
        if (artifact.kind === 'cron_trigger') return artifact.status === 'active';
        if (artifact.kind === 'skill_suggest') return artifact.status === 'pending';
        return false;
      })
      .map<IArtifactVO>((artifact) => ({
        type: 'artifact',
        id: `conversation-artifact:${artifact.conversation_artifact_id}`,
        artifact,
        created_at: artifact.created_at,
      }));

    if (visibleArtifacts.length === 0) {
      // Common streaming case: nothing to interleave, and `result` is already in
      // arrival (created_at) order — skip the O(n log n) re-sort that otherwise
      // runs on every streamed token and janks long conversations.
      return result;
    }
    return [...result, ...visibleArtifacts].toSorted(
      (a, b) => getProcessedItemCreatedAt(a) - getProcessedItemCreatedAt(b)
    );
  }, [artifacts, list]);

  const displayList = useMemo<IProcessedItem[]>(() => {
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
        processState: getProcessItemState(item),
        processStartedAt: getProcessedItemProcessStartedAt(item),
        processEndedAt: getProcessedItemProcessEndedAt(item),
        sourceMessageIds: getProcessedItemSourceMessageIds(item),
      };
    });
    const modelInput = assignTurnIdsFromUserRequests(rawModelInput, {
      activeTurnId: conversationContext?.activeTurnId,
      activeRequestMessageId: conversationContext?.activeRequestMessageId,
    });

    const disclosureItems = buildTurnDisclosureItems(modelInput, {
      tailClosed: conversationContext?.isProcessing !== true,
      activeTurnId: conversationContext?.activeTurnId,
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
          const summary = buildProcessReceiptSummary(item, state, t, workspaceRoots);
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
          };
        }

        const processItems = entry.processItemIds
          .map((id) => itemById.get(id))
          .filter((item): item is IRenderableItem => Boolean(item));

        return {
          type: 'turn_process_disclosure',
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

    // ── Live current-step strip: while the tail turn is still producing
    // output, append one synthetic row after the newest content so the user
    // can tell the task is running (the header reads "processed" throughout
    // the lifecycle). It disappears as soon as the turn settles. ──
    const isStreamingReplyText = (entry: IProcessedItem | undefined): boolean =>
      !!entry && 'type' in entry && entry.type === 'text' && (entry as IMessageText).position === 'left';

    const buildTurnLiveStep = (items: IProcessedItem[]): ITurnLiveStepVO | undefined => {
      if (conversationContext?.isProcessing !== true) return undefined;
      const tailDisclosure = items.findLast(
        (entry): entry is ITurnProcessDisclosureVO => 'type' in entry && entry.type === 'turn_process_disclosure'
      );
      if (!tailDisclosure) return undefined;
      const plan = planTurnLiveStep({
        isProcessing: true,
        disclosure: {
          running: tailDisclosure.running,
          processItems: tailDisclosure.processItems.map((processItem) => {
            const anchorId = getProcessedItemAnchorId(processItem);
            return {
              id: anchorId,
              state: tailDisclosure.processItemStates[anchorId] ?? getProcessItemState(processItem),
            };
          }),
        },
        hasStreamingReplyText: isStreamingReplyText(items.at(-1)),
      });
      if (!plan) return undefined;

      let label: string;
      let icon: TurnProcessReceiptIcon;
      if (plan.kind === 'item') {
        const processItem = tailDisclosure.processItems.find(
          (candidate) => getProcessedItemAnchorId(candidate) === plan.itemId
        );
        if (processItem && 'type' in processItem && processItem.type === 'thinking') {
          label = t('messages.processReceipt.thinkingRunning', { defaultValue: 'Thinking' });
          icon = 'thinking';
        } else if (processItem) {
          const summary = buildProcessReceiptSummary(processItem, plan.state, t, workspaceRoots);
          label = summary.label;
          icon = summary.icon;
        } else {
          label = t('messages.processReceipt.preparingAction', { defaultValue: 'Preparing next action' });
          icon = 'status';
        }
      } else if (plan.kind === 'composing') {
        label = t('messages.turnLiveStep.composing', { defaultValue: 'Composing the reply' });
        icon = 'status';
      } else if (plan.kind === 'analyzing') {
        label = t('messages.turnLiveStep.analyzing', { defaultValue: 'Analyzing the request' });
        icon = 'thinking';
      } else {
        label = t('messages.processReceipt.preparingAction', { defaultValue: 'Preparing next action' });
        icon = 'status';
      }

      return {
        type: 'turn_live_step',
        id: `turn-live-step-${tailDisclosure.msg_id}`,
        msg_id: tailDisclosure.msg_id,
        label,
        state: plan.state,
        icon,
        sourceMessageIds: [],
        created_at: tailDisclosure.endAt,
      };
    };

    // ── Turn deliverables: aggregate each successfully closed turn's verified
    // file artifacts and surface them as one card below that turn's last item
    // (its final assistant reply, when one exists). ──
    const turnGates = new Map<string, TurnGateInfo>();
    for (const entry of disclosureItems) {
      if ('type' in entry && entry.type === 'turn_process_disclosure') {
        turnGates.set(entry.msg_id, { running: entry.running, state: entry.state });
      }
    }

    const candidates: TurnDeliverableCandidate[] = [];
    for (const entry of modelInput) {
      const item = itemById.get(entry.id);
      if (!item) continue;
      const candidate: TurnDeliverableCandidate = {
        turnId: entry.turnId,
        role: entry.role,
        processState: entry.processState ?? 'completed',
      };
      if ('type' in item && item.type === 'tool_summary') {
        candidate.toolMessages = item.messages;
      } else if ('type' in item && item.type === 'file_summary') {
        candidate.fileDiffs = item.diffs;
        candidate.fileDiffSourceMessageIds = item.sourceMessageIds;
      }
      candidates.push(candidate);
    }

    const deliverablesByTurn = collectTurnDeliverables(candidates, { workspaceRoots, turnGates });
    const liveStepForDisclosures = buildTurnLiveStep(disclosureItems);
    if (deliverablesByTurn.size === 0) {
      return liveStepForDisclosures ? [...disclosureItems, liveStepForDisclosures] : disclosureItems;
    }

    const turnIdByAnchorId = new Map<string, MessageId | undefined>();
    for (const entry of modelInput) turnIdByAnchorId.set(entry.id, entry.turnId);
    const finalAssistantTextByTurn = new Map<MessageId, IMessageText>();
    for (const entry of modelInput) {
      if (!entry.turnId || entry.role !== 'assistant') continue;
      const item = itemById.get(entry.id);
      if (item?.type === 'text' && item.position === 'left') {
        finalAssistantTextByTurn.set(entry.turnId, item);
      }
    }
    const getDisplayItemTurnId = (entry: IProcessedItem): MessageId | undefined => {
      if ('type' in entry && entry.type === 'turn_process_disclosure') return entry.msg_id;
      if ('type' in entry && entry.type === 'process_receipt') return undefined;
      if ('type' in entry && entry.type === 'turn_deliverables') return entry.turn_id;
      if ('type' in entry && entry.type === 'turn_actions') return entry.turn_id;
      return turnIdByAnchorId.get(getProcessedItemAnchorId(entry));
    };

    const lastIndexByTurn = new Map<MessageId, number>();
    disclosureItems.forEach((entry, index) => {
      const turnId = getDisplayItemTurnId(entry);
      if (turnId && deliverablesByTurn.has(turnId)) lastIndexByTurn.set(turnId, index);
    });

    const withDeliverables: IProcessedItem[] = [];
    disclosureItems.forEach((entry, index) => {
      withDeliverables.push(entry);
      const turnId = getDisplayItemTurnId(entry);
      if (!turnId || lastIndexByTurn.get(turnId) !== index) return;
      const items = deliverablesByTurn.get(turnId);
      if (!items) return;
      withDeliverables.push({
        type: 'turn_deliverables',
        id: `turn-deliverables-${turnId}`,
        turn_id: turnId,
        items,
        sourceMessageIds: Array.from(
          new Set(items.flatMap((item) => item.sources.flatMap((source) => source.sourceMessageIds)))
        ),
        created_at: getProcessedItemCreatedAt(entry),
      });
      const actionMessage = finalAssistantTextByTurn.get(turnId);
      const actionMessageId = actionMessage ? getMessageBusinessIdentity(actionMessage) : undefined;
      if (actionMessage) {
        withDeliverables.push({
          type: 'turn_actions',
          id: `turn-actions-${turnId}`,
          turn_id: turnId,
          message: actionMessage,
          sourceMessageIds: actionMessageId ? [actionMessageId] : [],
          created_at: actionMessage.created_at ?? getProcessedItemCreatedAt(entry),
        });
      }
    });

    const liveStep = buildTurnLiveStep(withDeliverables);
    return liveStep ? [...withDeliverables, liveStep] : withDeliverables;
  }, [
    conversationContext?.activeRequestMessageId,
    conversationContext?.activeTurnId,
    conversationContext?.isProcessing,
    conversationContext?.stopNotice,
    processedList,
    t,
    workspaceRoots,
  ]);

  const lastUserTextIndex = useMemo(
    () =>
      displayList.findLastIndex(
        (item) =>
          !('type' in item &&
            ['turn_process_disclosure', 'process_receipt', 'artifact', 'turn_live_step'].includes(item.type)) &&
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
        ['turn_process_disclosure', 'process_receipt', 'artifact', 'turn_live_step'].includes(item.type)) &&
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

  // Use auto-scroll hook
  const {
    handleScrollerRef,
    handleContentRef,
    handleScroll,
    handleWheel,
    handlePointerDown,
    showScrollButton,
    scrollToBottom,
    scrollElementIntoView,
    hideScrollButton,
  } = useAutoScroll({
    messages: list,
    itemCount: displayList.length,
  });

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
      return;
    }

    handledTargetKeyRef.current = targetKey;
    setHighlightedMessageId(targetMessageId);
    hideScrollButton();

    requestAnimationFrame(() => {
      const targetElement = document.getElementById(`message-${getProcessedItemAnchorId(displayList[targetIndex])}`);
      scrollElementIntoView(targetElement, {
        behavior: 'smooth',
        block: 'center',
      });
    });

    const timer = window.setTimeout(() => {
      setHighlightedMessageId((current) => (current === targetMessageId ? undefined : current));
    }, 2400);

    return () => window.clearTimeout(timer);
  }, [displayList, hideScrollButton, location.key, scrollElementIntoView, targetMessageId]);

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
      if (targetIndex < 0) return;

      hideScrollButton();
      requestAnimationFrame(() => {
        const targetElement = document.getElementById(
          `message-${getProcessedItemAnchorId(displayList[targetIndex])}`
        );
        scrollElementIntoView(targetElement, {
          block: detail.align || 'start',
          behavior: detail.behavior || 'smooth',
        });
      });
    };

    window.addEventListener(CHAT_MESSAGE_JUMP_EVENT, handleMessageJump);
    return () => {
      window.removeEventListener(CHAT_MESSAGE_JUMP_EVENT, handleMessageJump);
    };
  }, [conversationContext?.conversation_id, displayList, hideScrollButton, scrollElementIntoView]);

  // Click scroll button
  const handleScrollButtonClick = () => {
    hideScrollButton();
    scrollToBottom('smooth');
  };

  const renderTurnDisclosure = (item: ITurnProcessDisclosureVO, highlighted: boolean) => {
    const getDisclosureProcessItemState = (processItem: IRenderableItem): TurnDisclosureProcessState =>
      item.processItemStates[getProcessedItemAnchorId(processItem)] ?? getProcessItemState(processItem);

    return (
      <TurnProcessDisclosure
        item={item}
        highlighted={highlighted}
        renderProcessItem={(processItem, expansionControls) =>
          renderProcessTraceItem(
            processItem,
            'list',
            workspaceRoots,
            getDisclosureProcessItemState(processItem),
            expansionControls
          )
        }
        getProcessItemKey={getProcessedItemAnchorId}
        getProcessItemState={getDisclosureProcessItemState}
        getProcessItemLayoutKind={getProcessItemLayoutKind}
        getProcessItemCanExpandAll={isCompletedThinkingProcessItem}
      />
    );
  };

  const renderProcessReceipt = (item: IProcessReceiptVO, highlighted: boolean) => {
    return (
      <TurnProcessReceipt
        receipt={item}
        highlighted={highlighted}
        renderProcessItem={(processItem) => renderProcessTraceItem(processItem, 'receipt', workspaceRoots)}
      />
    );
  };

  const renderItem = (_index: number, item: (typeof displayList)[0]) => {
    const highlighted = matchesTargetMessage(item, highlightedMessageId);
    if ('type' in item && item.type === 'turn_process_disclosure') {
      return (
        <div
          key={item.id}
          id={`message-${getProcessedItemAnchorId(item)}`}
          data-testid='turn-process-disclosure'
          className='min-w-0 message-item px-8px m-t-10px max-w-full md:max-w-780px mx-auto turn_process_disclosure'
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
          className='min-w-0 message-item px-8px m-t-10px max-w-full md:max-w-780px mx-auto process_receipt'
          style={highlighted ? highlightStyle : undefined}
        >
          {renderProcessReceipt(item, highlighted)}
        </div>
      );
    }
    if ('type' in item && item.type === 'artifact') {
      return (
        <div
          key={item.id}
          id={`message-${getProcessedItemAnchorId(item)}`}
          data-conversation-artifact-kind={item.artifact.kind}
          data-testid={`conversation-artifact-${item.artifact.kind}`}
          className='min-w-0 message-item px-8px m-t-10px max-w-full md:max-w-780px mx-auto'
          style={highlighted ? highlightStyle : undefined}
        >
          {item.artifact.kind === 'cron_trigger' ? (
            <MessageCronTrigger artifact={item.artifact} />
          ) : (
            <MessageSkillSuggest artifact={item.artifact} />
          )}
        </div>
      );
    }
    if ('type' in item && item.type === 'turn_deliverables') {
      return (
        <div
          key={item.id}
          id={`message-${getProcessedItemAnchorId(item)}`}
          data-testid='turn-deliverables'
          className='min-w-0 message-item px-8px m-t-10px max-w-full md:max-w-780px mx-auto turn_deliverables'
          style={highlighted ? highlightStyle : undefined}
        >
          <TurnDeliverablesCard
            items={item.items}
            workspace={conversationContext?.workspace}
            partial={hasMoreOlder === true}
          />
        </div>
      );
    }
    if ('type' in item && item.type === 'turn_actions') {
      return (
        <div
          key={item.id}
          id={`message-${getProcessedItemAnchorId(item)}`}
          data-testid='turn-actions'
          className='min-w-0 message-item px-8px max-w-full md:max-w-780px mx-auto turn_actions'
          style={highlighted ? highlightStyle : undefined}
        >
          <MessageText message={item.message} actionsOnly />
        </div>
      );
    }
    if ('type' in item && item.type === 'turn_live_step') {
      return (
        <div
          key={item.id}
          id={`message-${getProcessedItemAnchorId(item)}`}
          data-testid='turn-live-step'
          className='min-w-0 message-item px-8px m-t-10px max-w-full md:max-w-780px mx-auto turn_live_step'
        >
          <div className='turn-live-step'>
            <TurnProcessReceipt
              receipt={{
                id: item.id,
                item,
                label: item.label,
                state: item.state,
                icon: item.icon,
                defaultExpanded: false,
                hasDetail: false,
              }}
              renderProcessItem={() => null}
            />
          </div>
        </div>
      );
    }
    if ('type' in item && ['file_summary', 'tool_summary'].includes(item.type)) {
      return (
        <div
          key={item.id}
          id={`message-${getProcessedItemAnchorId(item)}`}
          className={'min-w-0 message-item px-8px m-t-10px max-w-full md:max-w-780px mx-auto ' + item.type}
          style={highlighted ? highlightStyle : undefined}
        >
          {renderProcessTraceItem(item, 'list', workspaceRoots)}
        </div>
      );
    }
    return (
      <MessageItem
        message={item as TMessage}
        key={(item as TMessage).id}
        highlighted={highlighted}
        hideActions={
          isActiveProcessTextItem(item, _index) ||
          movedActionMessageIds.has((item as TMessage).id)
        }
      ></MessageItem>
    );
  };

  if (displayList.length === 0 && isMessageListLoading) {
    return <MessageListSkeleton />;
  }

  if (displayList.length === 0 && emptySlot) {
    return <div className='relative flex-1 h-full flex items-center justify-center'>{emptySlot}</div>;
  }

  return (
    <div className='relative flex-1 h-full'>
      <ConversationQuestionLocator conversation_id={conversationContext?.conversation_id} />

      {/* Use PreviewGroup to wrap all messages for cross-message image preview */}
      <Image.PreviewGroup actionsLayout={['zoomIn', 'zoomOut', 'originalSize', 'rotateLeft', 'rotateRight']}>
        <ImagePreviewContext.Provider value={{ inPreviewGroup: true }}>
          <div
            ref={handleScrollerRef}
            data-testid='message-list-scroller'
            className='flex-1 h-full overflow-y-auto pb-10px box-border'
            style={{ overflowAnchor: 'none' }}
            onPointerDown={handlePointerDown}
            onScroll={handleScrollWithPaging}
            onWheel={handleWheel}
          >
            <div ref={handleContentRef} data-testid='message-list-content' style={{ overflowAnchor: 'none' }}>
              <div className='h-10px' />
              {displayList.map((item, index) => (
                <React.Fragment key={item.id}>{renderItem(index, item)}</React.Fragment>
              ))}
              <div className='h-20px' />
            </div>
          </div>
        </ImagePreviewContext.Provider>
      </Image.PreviewGroup>

      {showScrollButton && (
        <>
          {/* Gradient mask */}
          <div className='absolute bottom-0 left-0 right-0 h-100px pointer-events-none' />
          {/* Scroll button */}
          <div className='absolute bottom-20px left-50% transform -translate-x-50% z-100'>
            <div
              className='flex items-center justify-center w-40px h-40px rd-full bg-base shadow-lg cursor-pointer hover:bg-1 transition-all hover:scale-110 border-1px border-solid border-3'
              onClick={handleScrollButtonClick}
              title={t('messages.scrollToBottom')}
              style={{ lineHeight: 0 }}
            >
              <Down theme='filled' size='20' fill={iconColors.secondary} style={{ display: 'block' }} />
            </div>
          </div>
        </>
      )}

      <SelectionReplyButton messages={list} />
    </div>
  );
};

export default MessageList;
