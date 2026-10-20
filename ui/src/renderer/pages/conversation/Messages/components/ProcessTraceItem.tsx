import type { IMessageToolCall, IMessageToolGroup, TMessage } from '@/common/chat/chatLib';
import { toDisplayText } from '@/common/chat/displayText';
import { normalizeToolMessages } from '@/common/chat/normalizeToolCall';
import type { ToolDetailIdentity } from '@/common/chat/toolDetails';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { hasSkillSuggest, stripSkillSuggest } from '@renderer/utils/chat/skillSuggestParser';
import { hasThinkTags, stripThinkTags } from '@renderer/utils/chat/thinkTagFilter';
import { Attention, Code, Edit, FolderOpen, Right, Terminal } from '@icon-park/react';
import classNames from 'classnames';
import { Spin } from '@arco-design/web-react';
import React, { useContext, useId, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { isContextCompressionTip } from '../processTipModel';
import { formatWorkspaceFileTarget } from '../processFileTargetLabel';
import { hasVisibleProcessReasoning, projectAssistantText, stripInternalToolCallPayload } from '../processTraceDisplayModel';
import { ProcessDetailsVisibilityContext } from '../processDetailsVisibility';
import ToolDetails from './ToolDetails';
import type { TurnDisclosureProcessState } from '../turnDisclosureModel';
import type { MessageId } from '@/common/types/ids';
import { getProcessItemState } from '../turnProcessState';
import { isToolObservationUnconfirmed } from '../processSegments';
import AssistantProtocolNotice from './AssistantProtocolNotice';
import MessageThinking from './MessageThinking';
import ConversationProse from './ConversationProse';
import { MESSAGE_BODY_FONT_SIZE, MESSAGE_BODY_LINE_HEIGHT } from '../typography';
import { buildToolReceiptRows, type ToolReceiptRow } from './toolReceiptModel';
import FileActivitySummary, { FileActivityList } from './FileActivitySummary';
import { isConfirmedFileActivity } from '../fileActivityModel';

type ToolProcessMessage = IMessageToolGroup | IMessageToolCall;
export type ProcessTraceRenderableItem = TMessage | {
  type: 'tool_summary'; id: string; msg_id?: MessageId; messages: ToolProcessMessage[];
  sourceMessageIds: string[]; created_at: number;
};
type TranslationFn = ReturnType<typeof useTranslation>['t'];
const getPublicProcessNarration = (value: unknown): string => {
  let content = toDisplayText(value).trim();
  if (hasThinkTags(content)) content = stripThinkTags(content);
  if (hasSkillSuggest(content)) content = stripSkillSuggest(content);
  return stripInternalToolCallPayload(content);
};

const getToolReceiptDetailDisplayTarget = (row: ToolReceiptRow, workspaceRoots: string[]): string | undefined => {
  if (!row.target) return undefined;
  if (row.action !== 'read_files' && row.action !== 'edit_files') return row.target;
  return formatWorkspaceFileTarget(row.target, { workspaceRoots }).label;
};

const formatToolReceiptDetailLabel = (
  row: ToolReceiptRow,
  t: TranslationFn,
  workspaceRoots: string[]
): string => {
  const displayTarget = getToolReceiptDetailDisplayTarget(row, workspaceRoots);


  if (row.skipped) {
    return t('messages.toolSummary.skipped', {
      target: displayTarget ?? row.title,
      defaultValue: 'Skipped {{target}}',
    });
  }

  if (row.notExecutedReason === 'invalid_arguments') {
    return t('messages.toolSummary.invalidArguments', {
      target: displayTarget ?? row.title,
      defaultValue: 'Arguments did not pass validation; {{target}} was not run',
    });
  }

  if (row.notExecutedReason === 'runtime_preflight') {
    return t('messages.toolSummary.notExecuted', {
      target: displayTarget ?? row.title,
      defaultValue: 'Did not run {{target}}',
    });
  }
  if (row.notExecutedReason === 'process_reference') {
    return t('messages.toolSummary.processReferenceInvalid', {
      target: displayTarget ?? row.title,
      defaultValue: 'Process not found in this execution; did not run {{target}}',
    });
  }

  if (row.commandNotStarted) {
    return t('messages.toolSummary.commandNotStarted', {
      target: displayTarget ?? row.title,
      defaultValue: '{{target}} did not start; command was not run',
    });
  }

  if (row.commandTimedOut) {
    return t('messages.toolSummary.commandTimedOut', {
      target: displayTarget ?? row.title,
      defaultValue: '{{target}} reached its time limit; process cleanup completed',
    });
  }

  if (row.commandExitCode !== undefined) {
    return t('messages.toolSummary.commandExited', {
      target: displayTarget ?? row.title, code: row.commandExitCode,
      defaultValue: '{{target}} ended with exit code {{code}}',
    });
  }

  return row.action === 'generic' ? row.target ?? row.title
    : [row.title, displayTarget && displayTarget !== row.title ? displayTarget : undefined].filter(Boolean).join(' · ');
};


const DetailSection: React.FC<{ label: string; value?: string }> = ({ label, value }) => value ?
  <div className='turn-process-trace-detail__section'>
    <div className='turn-process-trace-detail__label'>{label}</div>
    <pre className='turn-process-trace-detail__content'>{value}</pre>
  </div> : null;

type ToolObservationRow = ToolReceiptRow & { observationUnconfirmed?: boolean; detailSource?: ToolDetailIdentity; confirmedFile?: boolean; fileGroupId?: string };

const ToolTraceRow: React.FC<{ row: ToolObservationRow; workspaceRoots: string[] }> = ({ row, workspaceRoots }) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const parentVisible = useContext(ProcessDetailsVisibilityContext);
  const detailId = useId();
  const hasPayload = row.detailPolicy === 'payload' && Boolean(row.detailSource);
  const hasDetail = Boolean(hasPayload || row.error || row.detailPolicy === 'files' && row.fileSummary);
  const label = row.observationUnconfirmed
    ? `${row.target ?? row.title} · ${t('messages.toolResultUnconfirmed')}`
    : formatToolReceiptDetailLabel(row, t, workspaceRoots);
  const visualState = row.observationUnconfirmed ? 'unconfirmed' : row.state;
  const Header = hasDetail ? 'button' : 'div';
  return <div className='turn-process-trace-tool' data-tool-call-id={row.key} data-tool-observation={row.observationUnconfirmed ? 'unconfirmed' : undefined}>
    <Header type={hasDetail ? 'button' : undefined}
      className={classNames('turn-process-trace__row turn-process-trace-tool__toggle', `turn-process-trace__row--${visualState}`)}
      onClick={hasDetail ? () => setExpanded(value => !value) : undefined}
      aria-expanded={hasDetail ? expanded : undefined} aria-controls={hasDetail ? detailId : undefined}>
      <span className='turn-process-trace__row-icon' role={row.state === 'failed' || row.state === 'canceled' ? 'img' : undefined}
        aria-label={row.state === 'failed' || row.state === 'canceled' ? t(`messages.toolSummary.${row.state}`, { target: label }) : undefined}>
        {row.state === 'running' ? <Spin size={12} /> : row.state === 'failed' || row.state === 'canceled' ? <Attention theme='outline' size='13' />
          : row.action === 'edit_files' ? <Edit theme='outline' size='13' /> : row.action === 'read_files' || row.action === 'search_code' ? <FolderOpen theme='outline' size='13' />
          : row.action === 'run_commands' ? <Terminal theme='outline' size='13' /> : <Code theme='outline' size='13' />}
      </span>
      <span className='turn-process-trace__text'>{label}</span>
      {row.foreground && <span className='text-11px text-t-secondary'>{t('messages.computerForeground')}</span>}
      {hasDetail && <Right theme='outline' size='12' className={classNames('turn-process-trace-tool__arrow', expanded && 'turn-process-trace-tool__arrow--open')} />}
    </Header>
    {expanded && parentVisible && hasDetail && <div id={detailId} className='turn-process-trace-detail'>
      {row.observationUnconfirmed && <div className='turn-process-trace-detail__label'>{t('messages.toolResultUnconfirmed')}</div>}
      <DetailSection label={t('messages.toolDetailError')} value={row.error} />
      {row.detailPolicy === 'files' && row.fileSummary && <FileActivityList items={[{ key: row.key, summary: row.fileSummary }]}
        workspaceRoots={workspaceRoots} confirmed={row.confirmedFile === true} />}
      {hasPayload && row.detailSource && <ToolDetails source={row.detailSource} />}
    </div>}
  </div>;
};

const ToolProcessTraceRows: React.FC<{ messages: ToolProcessMessage[]; workspaceRoots: string[]; stateOverride?: TurnDisclosureProcessState }> = ({ messages, workspaceRoots, stateOverride }) => {
  const { i18n } = useTranslation();
  const language = i18n.resolvedLanguage ?? i18n.language;
  const rows = useMemo(() => buildToolReceiptRows(normalizeToolMessages(messages), language).map((row): ToolObservationRow => {
    const observationUnconfirmed = messages.some(message => isToolObservationUnconfirmed(message, row.key));
    const source = messages.find(message => message.type === 'tool_call' ? message.content.call_id === row.key
      : message.content.some(tool => tool.call_id === row.key));
    const detailSource: ToolDetailIdentity | undefined = source?.turn_id ? {
      conversation_id: source.conversation_id, turn_id: source.turn_id, call_id: row.key,
      ...(source.type === 'tool_call' && source.content.timeline ? { through_seq: Math.max(source.content.timeline.through_seq, source.content.result_seq ?? 0) } : {}),
      ...(source.type === 'tool_call' && source.content.result_seq !== undefined ? { expected_result_seq: source.content.result_seq } : {}),
    } : undefined;
    const view = { ...row, detailSource, confirmedFile: source?.type === 'tool_call' && isConfirmedFileActivity(source),
      fileGroupId: source?.type === 'tool_call' ? source.content.timeline?.segments.find(segment => segment.kind === 'tool' && segment.call_id === row.key)?.group_id : undefined };
    return observationUnconfirmed ? { ...view, state: 'completed', observationUnconfirmed: true }
      : stateOverride && row.state === 'running' ? { ...view, state: stateOverride } : view;
  }), [language, messages, stateOverride]);
  const entries: Array<ToolObservationRow | { key: string; files: ToolObservationRow[] }> = [];
  for (const row of rows) {
    const previous = entries.at(-1);
    if (row.confirmedFile && row.fileSummary) {
      if (previous && 'files' in previous && previous.files[0].detailSource?.conversation_id === row.detailSource?.conversation_id
        && previous.files[0].detailSource?.turn_id === row.detailSource?.turn_id && previous.files[0].fileGroupId === row.fileGroupId) previous.files.push(row);
      else entries.push({ key: row.key, files: [row] });
    } else entries.push(row);
  }
  return <div className='turn-process-trace'>{entries.map(entry => 'files' in entry
    ? <FileActivitySummary key={entry.key} items={entry.files.flatMap(row => row.fileSummary ? [{ key: row.key, summary: row.fileSummary }] : [])} workspaceRoots={workspaceRoots} />
    : <ToolTraceRow key={entry.key} row={entry} workspaceRoots={workspaceRoots} />)}</div>;
};

const ProcessTraceItem: React.FC<{ item: ProcessTraceRenderableItem; workspaceRoots?: string[];
  stateOverride?: TurnDisclosureProcessState }> = ({ item, workspaceRoots, stateOverride }) => {
  const { t } = useTranslation();
  const context = useConversationContextSafe();
  const roots = workspaceRoots?.length ? workspaceRoots : context?.workspace ? [context.workspace] : [];
  const state = stateOverride ?? getProcessItemState(item);
  if (item.type === 'tool_summary') return <ToolProcessTraceRows messages={item.messages} workspaceRoots={roots} stateOverride={stateOverride} />;
  if (item.type === 'tool_call' || item.type === 'tool_group') return <ToolProcessTraceRows messages={[item]} workspaceRoots={roots} stateOverride={stateOverride} />;
  if (item.type === 'thinking') {
    const content = toDisplayText(item.content.content).trim();
    return !hasVisibleProcessReasoning(content) ? null :
      <MessageThinking message={item} variant='process' completed={state !== 'running'} />;
  }
  if (item.type === 'text') {
    const content = getPublicProcessNarration(item.content.content);
    const protocol = projectAssistantText(toDisplayText(item.content.content)).hasToolPayload;
    if (!content && !protocol) return null;
    return <div className='turn-process-trace__narration'>
      {content && <div className='turn-process-trace__paragraph' data-testid='process-narration'><ConversationProse message={item} text={content} variant='process' fontSize={MESSAGE_BODY_FONT_SIZE} lineHeight={MESSAGE_BODY_LINE_HEIGHT} /></div>}
      {protocol && <AssistantProtocolNotice />}
    </div>;
  }
  if (item.type === 'agent_status' && item.content.turn_summary) return null;
  const label = item.type === 'tips' ? isContextCompressionTip(item) ? t('messages.processReceipt.contextCompressed') : toDisplayText(item.content.content)
    : item.content.status === 'preparing' ? t('messages.processReceipt.preparingAction') : t('messages.processReceipt.preparedAction');
  return <div className='turn-process-trace__row'><Code theme='outline' size='13' className='turn-process-trace__row-icon' /><span className='turn-process-trace__text'>{label}</span></div>;
};
export default ProcessTraceItem;
