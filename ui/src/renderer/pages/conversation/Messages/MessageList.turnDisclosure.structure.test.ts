/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./MessageList.tsx', import.meta.url), 'utf8');
const buildSummarySource = source.slice(
  source.indexOf('const buildProcessReceiptSummary'),
  source.indexOf('const highlightStyle')
);

describe('MessageList turn completion disclosure structure', () => {
  test('routes message content through the turn disclosure model before rendering', () => {
    expect(source.includes('buildTurnDisclosureItems')).toBe(true);
    expect(source.includes('assignTurnIdsFromUserRequests')).toBe(true);
    expect(source.includes('tailClosed: conversationContext?.isProcessing !== true')).toBe(true);
    expect(source.includes('activeTurnId: conversationContext?.activeTurnId')).toBe(true);
    expect(source.includes("type: 'turn_process_disclosure'")).toBe(true);
    expect(source.includes('renderTurnDisclosure')).toBe(true);
    expect(source.includes('components/TurnProcessDisclosure')).toBe(true);
    expect(source.includes("type: 'process_receipt'")).toBe(true);
    expect(source.includes('renderProcessReceipt')).toBe(true);
    expect(source.includes('components/TurnProcessReceipt')).toBe(true);
    expect(source.includes('components/ProcessTraceItem')).toBe(true);
    expect(source.includes('renderProcessTraceItem')).toBe(true);
    expect(source.includes('getProcessItemState')).toBe(true);
    expect(source.includes('highlighted={highlighted}')).toBe(true);
  });

  test('uses the stable view-model id as the React reconciliation key', () => {
    expect(source.includes('<React.Fragment key={item.id}>')).toBe(true);
    expect(source.includes('<React.Fragment key={getProcessedItemAnchorId(item) || index}>')).toBe(false);
  });

  test('places conversation creation cards at the end of their canonical turn', () => {
    expect(source.includes('useConversationCreationTaskOwnerMessageIds')).toBe(true);
    expect(source.includes('creationTaskPlacementAfterIndices')).toBe(true);
    expect(source.includes("type: 'turn_creation_tasks'")).toBe(true);
    expect(source.includes('<ConversationCreationTaskCards messageId={item.message_id} />')).toBe(true);
    expect(source.includes("'position' in item && item.position === 'right'")).toBe(false);
  });

  test('renders chronological journal receipts without legacy process cards', () => {
    expect(source.includes('renderProcessTraceItem(')).toBe(true);
    expect(source.includes('renderJournalProcessItem')).toBe(true);
    expect(source.includes('journal-receipt-')).toBe(true);
    expect(source.includes('<TurnProcessReceipt')).toBe(true);
    expect(source.includes('MessageToolGroupSummary')).toBe(false);
    expect(source.includes('defaultExpanded={true}')).toBe(false);
  });

  test('preserves process arrival order instead of collapsing narration or tool batches', () => {
    expect(source.includes('collapseProcessNarration')).toBe(false);
    expect(source.includes('coalesceToolProcessSummaries')).toBe(false);
    expect(source.includes('deduplicateProcessText')).toBe(false);
    expect(source.includes('item={item}')).toBe(true);
    expect(source.includes('finalText: item.finalAnswer')).toBe(true);
  });

  test('summarizes each adjacent tool stage behind one localized disclosure row', () => {
    expect(source.includes('const summarySeparator = t(')).toBe(true);
    expect(source.includes("messages.processReceipt.summarySeparator")).toBe(true);
    expect(source.includes('defaultExpanded: false')).toBe(true);
    expect(source.includes('hasDetail: true')).toBe(true);
  });

  test('keeps thinking in the process disclosure content without turning it into a receipt', () => {
    expect(source.includes("case 'thinking':\n      return 'process_content';")).toBe(true);
    expect(source.includes("case 'thinking':\n    case 'tool_call':")).toBe(false);
    expect(source.includes('renderProcessTraceItem(processItem')).toBe(true);
  });

  test('renders thinking through the process trace body instead of process receipts', () => {
    const renderProcessReceiptSource =
      source.match(/const renderProcessReceipt = \(item: IProcessReceiptVO, highlighted: boolean\) => \{[\s\S]*?  \};/)?.[0] ?? '';

    expect(source.includes('isReadableThinkingReceipt')).toBe(false);
    expect(source.includes("if (isReadableThinkingReceipt(item)) {")).toBe(false);
    expect(renderProcessReceiptSource.includes('<TurnProcessReceipt')).toBe(true);
    expect(source.includes("case 'thinking':\n        return <MessageThinking message={message}></MessageThinking>;")).toBe(true);
    expect(source.includes('isProcessTraceRenderableItem')).toBe(false);
  });

  test('suppresses copy and timestamp actions for active process text', () => {
    expect(source.includes('isActiveProcessTextItem')).toBe(true);
    expect(source.includes('lastUserTextIndex')).toBe(true);
    expect(source.includes("conversationContext?.isProcessing === true")).toBe(true);
    expect(source.includes('<MessageText message={message} hideActions={hideActions}></MessageText>')).toBe(true);
    expect(source.includes('isActiveProcessTextItem(item, _index) ||')).toBe(true);
    expect(source.includes('movedActionMessageIds.has((item as TMessage).id)')).toBe(true);
  });

  test('passes closed-turn effective process state into disclosure details', () => {
    expect(source.includes('processItemStates: Record<string, TurnDisclosureProcessState>')).toBe(true);
    expect(source.includes('processItemStates: entry.processItemStates')).toBe(true);
    expect(source.includes('getDisclosureProcessItemState')).toBe(true);
    expect(source.includes('item.running ? undefined : processState')).toBe(true);
    expect(source.includes('recoverFailures={recoverFailures}')).toBe(true);
  });

  test('uses canonical timing metadata without rendering a redundant status row', () => {
    expect(source.includes('getProcessedItemTurnStartedAt')).toBe(true);
    expect(source.includes('getProcessedItemTurnEndedAt')).toBe(true);
    expect(source.includes("item.content.turn_summary ? 'metadata' : 'process'")).toBe(true);
    expect(source.includes('turnStartedAt: getProcessedItemTurnStartedAt(item)')).toBe(true);
    expect(source.includes('turnEndedAt: getProcessedItemTurnEndedAt(item)')).toBe(true);
  });

  test('keeps model activity receipts as static single-line status rows', () => {
    const agentStatusCase = buildSummarySource.match(/case 'agent_status':[\s\S]*?case 'tips':/)?.[0] ?? '';

    expect(agentStatusCase.includes("item.content.status === 'preparing'")).toBe(true);
    expect(agentStatusCase.includes("item.content.status === 'prepared'")).toBe(true);
    expect(agentStatusCase.includes('hasDetail: false')).toBe(true);
  });

  test('marks only genuinely detailed receipts as expandable', () => {
    const toolSummaryCase =
      buildSummarySource.match(/if \('type' in item && item\.type === 'tool_summary'\) \{[\s\S]*?if \('type' in item && item\.type === 'file_summary'\)/)?.[0] ?? '';
    const fileSummaryCase =
      buildSummarySource.match(/if \('type' in item && item\.type === 'file_summary'\) \{[\s\S]*?switch \(item\.type\)/)?.[0] ?? '';
    expect(toolSummaryCase.includes('hasDetail: true')).toBe(true);
    expect(fileSummaryCase.includes('hasDetail: item.diffs.length > 1')).toBe(true);
  });

  test('routes context compaction tips through process receipts instead of assistant text', () => {
    expect(source.includes('isContextCompressionTip')).toBe(true);
    expect(source.includes("if (isContextCompressionTip(item)) return 'process';")).toBe(true);
  });

  test('renders barrier-skipped receipt summaries with dedicated copy', () => {
    expect(source.includes('part.skipped')).toBe(true);
    expect(source.includes('messages.processReceipt.skippedAfterFailure')).toBe(true);
  });

  test('renders pre-dispatch argument rejection with dedicated neutral copy', () => {
    expect(source.includes("part.notExecutedReason === 'invalid_arguments'")).toBe(true);
    expect(source.includes('messages.processReceipt.invalidArguments')).toBe(true);
    expect(source.includes('messages.processReceipt.commandNotExecuted')).toBe(true);
  });

  test('uses plan events as hard boundaries between tool receipt groups', () => {
    const planBoundary = source.match(/if \(message\.type === 'plan'\) \{[\s\S]*?continue;[\s\S]*?\}/)?.[0] ?? '';

    expect(planBoundary.includes('toolList = [];')).toBe(true);
    expect(planBoundary.includes('toolSourceMessageIds = [];')).toBe(true);
    expect(planBoundary.includes('diffsChanges = [];')).toBe(true);
    expect(planBoundary.includes('diffsSourceMessageIds = [];')).toBe(true);
  });

  test('suppresses only legacy synthetic plan-tool failures with a persisted plan projection', () => {
    expect(source.includes("from './planToolVisibility'")).toBe(true);
    expect(source.includes('isSupersededPlanToolFailure(message, journalSources.slice(i + 1))')).toBe(true);
  });
});
