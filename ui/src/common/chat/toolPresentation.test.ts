import { describe, expect, test } from 'bun:test';
import { normalizeToolCall } from './normalizeToolCall';
import { formatToolDiagnostics, formatToolPresentationLabel, resolveToolPresentation } from './toolPresentation';
import type { IMessageToolCall } from './chatLib';
import { parseConversationId } from '../types/ids';

describe('tool presentation', () => {
  test('live aliases and historical canonical identities share the same title and target', () => {
    const historicalName = 'platform__web_research_web_research_search__3975312c8d0828c33bc7';
    const makeCall = (name: string) => normalizeToolCall({ id: 'web', type: 'tool_call',
      conversation_id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000051'), content: {
      call_id: 'web', name, capability_id: 'web.research', action_id: 'web.research/search',
      status: 'completed', args: { query: '王者荣耀英雄属性' },
    } } satisfies IMessageToolCall)!;
    const historical = makeCall(historicalName);
    const live = makeCall('web_search');
    expect(historical.name).toBe(historicalName);
    expect(formatToolPresentationLabel(resolveToolPresentation(historical, 'zh-CN')))
      .toBe('搜索网页 · 王者荣耀英雄属性');
    expect(formatToolPresentationLabel(resolveToolPresentation(live, 'zh-CN')))
      .toBe('搜索网页 · 王者荣耀英雄属性');
    expect(formatToolDiagnostics(resolveToolPresentation(historical))).toContain(historicalName);
    expect(formatToolDiagnostics(resolveToolPresentation(live))).toContain('web.research/search');
  });

  test('uses the page domain as a compact target and keeps its full URL in input', () => {
    const input = JSON.stringify({ url: 'https://pvp.qq.com/ingame/kis/hero.shtml?query=123' });
    const presentation = resolveToolPresentation({ name: 'web_fetch', input }, 'zh-Hans');
    expect(formatToolPresentationLabel(presentation)).toBe('读取网页 · pvp.qq.com');
    expect(input).toContain('/ingame/kis/hero.shtml?query=123');
    expect(resolveToolPresentation({ name: 'web_fetch', input }, 'en-US').title).toBe('Read a web page');
  });

  test('retains distinct origins when external tools share the same human title', () => {
    const first = resolveToolPresentation({ name: 'mcp__alpha__search_documents__0123456789abcdefabcd' });
    const second = resolveToolPresentation({ name: 'mcp__beta__search_documents__abcdef0123456789abcd' });
    expect(first.title).toBe('Search documents');
    expect(second.title).toBe(first.title);
    expect(first.source).toBe('MCP · alpha');
    expect(second.source).toBe('MCP · beta');
    expect(formatToolDiagnostics(first)).not.toBe(formatToolDiagnostics(second));
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
    const presentation = resolveToolPresentation({ name: 'web_search', input: JSON.stringify({ query: '搜'.repeat(200) }) });
    expect(presentation.target).toBe('搜'.repeat(120) + '…');
    expect(resolveToolPresentation({ name: 'web_search', input: 'bad json' }).target).toBeUndefined();
    expect(resolveToolPresentation({ name: 'platform__truncated_slug__0123456789abcdefabcd' }, 'zh-CN').title)
      .toBe('工具调用');
    expect(resolveToolPresentation({ name: 'plugin__calendar__create_event__0123456789abcdefabcd' }).title)
      .toBe('Create event');
  });
});
