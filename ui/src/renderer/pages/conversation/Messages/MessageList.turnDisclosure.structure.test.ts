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
  test('keeps unclassified tool details inspectable without inventing process boundaries', () => {
    expect(source.includes('const summarySeparator = t(')).toBe(true);
    expect(source.includes("messages.processReceipt.summarySeparator")).toBe(true);
    expect(source.includes('defaultExpanded: false')).toBe(true);
    expect(source.includes('hasDetail: true')).toBe(true);
  });

  test('suppresses copy and timestamp actions for active process text', () => {
    expect(source.includes('isActiveProcessTextItem')).toBe(true);
    expect(source.includes('lastUserTextIndex')).toBe(true);
    expect(source.includes("conversationContext?.isProcessing === true")).toBe(true);
    expect(source.includes('<MessageText message={message} hideActions={hideActions}></MessageText>')).toBe(true);
    expect(source.includes('isActiveProcessTextItem(item, _index) ||')).toBe(true);
    expect(source.includes('movedActionMessageIds.has((item as TMessage).id)')).toBe(true);
  });

  test('keeps model activity receipts as static single-line status rows', () => {
    const agentStatusCase = buildSummarySource.match(/case 'agent_status':[\s\S]*?case 'tips':/)?.[0] ?? '';

    expect(agentStatusCase.includes("item.content.status === 'preparing'")).toBe(true);
    expect(agentStatusCase.includes("item.content.status === 'prepared'")).toBe(true);
    expect(agentStatusCase.includes('hasDetail: false')).toBe(true);
  });

  test('marks only genuinely detailed receipts as expandable', () => {
    const toolSummaryCase =
      buildSummarySource.match(/if \('type' in item && item\.type === 'tool_summary'\) \{[\s\S]*?switch \(item\.type\)/)?.[0] ?? '';
    expect(toolSummaryCase.includes('hasDetail: true')).toBe(true);
  });

  test('routes context compaction tips through process receipts instead of assistant text', () => {
    expect(source.includes('isContextCompressionTip')).toBe(true);
    expect(source.includes("if (isContextCompressionTip(item)) return 'process';")).toBe(true);
  });






});
