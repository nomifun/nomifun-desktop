import { describe, expect, test } from 'bun:test';
import { normalizeToolCall } from './normalizeToolCall';
import { formatToolPresentationLabel, isNativeCompletionReport, resolveToolPresentation } from './toolPresentation';
import type { IMessageToolCall } from './chatLib';
import { parseConversationId } from '../types/ids';

describe('tool presentation', () => {
  test('live aliases and historical canonical identities share the same title and target', () => {
    const historicalName = 'platform__web_research_web_research_search__3975312c8d0828c33bc7';
    const makeCall = (name: string) => normalizeToolCall({ id: 'web', type: 'tool_call',
      conversation_id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000051'), content: {
      call_id: 'web', name, capability_id: 'web.research', action_id: 'web.research/search',
      status: 'completed', description: '王者荣耀英雄属性',
    } } satisfies IMessageToolCall)!;
    const historical = makeCall(historicalName);
    const live = makeCall('web_search');
    expect(historical.name).toBe(historicalName);
    expect(formatToolPresentationLabel(resolveToolPresentation(historical, 'zh-CN')))
      .toBe('搜索网页 · 王者荣耀英雄属性');
    expect(formatToolPresentationLabel(resolveToolPresentation(live, 'zh-CN')))
      .toBe('搜索网页 · 王者荣耀英雄属性');
  });

  test('uses the host-projected page domain without retaining URL arguments', () => {
    const presentation = resolveToolPresentation({ name: 'web_fetch', description: 'pvp.qq.com' }, 'zh-Hans');
    expect(formatToolPresentationLabel(presentation)).toBe('读取网页 · pvp.qq.com');
    expect(resolveToolPresentation({ name: 'web_fetch', description: 'pvp.qq.com' }, 'en-US').title).toBe('Read a web page');
  });

  test('retains distinct origins when external tools share the same human title', () => {
    const first = resolveToolPresentation({ name: 'mcp__alpha__search_documents__0123456789abcdefabcd' });
    const second = resolveToolPresentation({ name: 'mcp__beta__search_documents__abcdef0123456789abcd' });
    expect(first.title).toBe('Search documents');
    expect(second.title).toBe(first.title);
    expect(first.source).toBe('MCP · alpha');
    expect(second.source).toBe('MCP · beta');
  });

  test('does not reinterpret an explicit external identity as a native action', () => {
    const presentation = resolveToolPresentation({ name: 'web_search',
      capabilityId: 'third.party', actionId: 'custom_action' }, 'zh-CN');
    expect(presentation.title).toBe('Web search');
    expect(presentation.receiptAction).toBeUndefined();
    expect(resolveToolPresentation({ name: 'web_search',
      origin: { kind: 'mcp', name: 'External', toolName: 'web_search' } }, 'zh-CN').title).toBe('Web search');
  });

  test('bounds summaries without losing diagnostics or throwing on malformed inputs', () => {
    const presentation = resolveToolPresentation({ name: 'web_search', description: '搜'.repeat(200) });
    expect(presentation.target).toBe('搜'.repeat(120) + '…');
    expect(resolveToolPresentation({ name: 'web_search', description: undefined }).target).toBeUndefined();
    expect(resolveToolPresentation({ name: 'platform__truncated_slug__0123456789abcdefabcd' }, 'zh-CN').title)
      .toBe('工具调用');
    expect(resolveToolPresentation({ name: 'plugin__calendar__create_event__0123456789abcdefabcd' }).title)
      .toBe('Create event');
  });

  test('only the exact reserved completion control gets receipt-only details', () => {
    expect(isNativeCompletionReport({ name: 'report_completion' })).toBe(true);
    expect(resolveToolPresentation({ name: 'report_completion', description: 'Duplicate final answer' }).detailPolicy).toBe('receipt');
    for (const identity of [{ capabilityId: 'external' }, { actionId: 'external/report' }, { capabilityId: '' },
      { actionId: '' }, { capabilityId: 'external', actionId: 'external/report' },
      { origin: { kind: 'mcp' as const, name: 'Server', toolName: 'report_completion' } }]) {
      const input = { name: 'report_completion', ...identity };
      expect(isNativeCompletionReport(input)).toBe(false);
      expect(resolveToolPresentation(input).detailPolicy).toBe('payload');
    }
    for (const name of ['mcp__server__report_completion__0123456789abcdefabcd', 'plugin__server__report_completion__0123456789abcdefabcd', 'Report_completion']) {
      expect(isNativeCompletionReport({ name })).toBe(false);
      expect(resolveToolPresentation({ name }).detailPolicy).toBe('payload');
    }
  });

  test('file operations expose lists while command, search and ordinary tools retain payload details', () => {
    for (const name of ['read_file', 'write_file', 'apply_patch', 'delete_path', 'rename_path']) {
      expect(resolveToolPresentation({ name }).detailPolicy).toBe('files');
    }
    expect(resolveToolPresentation({ name: 'platform__alias', capabilityId: 'workspace.files', actionId: 'workspace.files/read' }).detailPolicy).toBe('files');
    for (const actionId of ['ssh/fs.read', 'ssh/fs.write']) {
      const presentation = resolveToolPresentation({ name: 'platform__remote', capabilityId: 'ssh', actionId });
      expect(presentation.detailPolicy).toBe('files');
      expect(presentation.receiptAction).toBeUndefined();
    }
    for (const name of ['start_process', 'search_files', 'web_fetch', 'mcp__server__read_file__0123456789abcdefabcd']) {
      expect(resolveToolPresentation({ name }).detailPolicy).toBe('payload');
    }
    for (const identity of [{ capabilityId: 'external' }, { actionId: 'external/read' }, { capabilityId: 'external', actionId: 'external/read' }]) {
      const presentation = resolveToolPresentation({ name: 'read_file', ...identity });
      expect(presentation.detailPolicy).toBe('payload');
      expect(presentation.receiptAction).toBeUndefined();
    }
    expect(resolveToolPresentation({ name: 'write_file', origin: { kind: 'plugin', name: 'Writer', toolName: 'write_file' } }).detailPolicy).toBe('payload');
  });
});
