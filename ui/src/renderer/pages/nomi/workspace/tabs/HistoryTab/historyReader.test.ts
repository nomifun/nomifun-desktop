/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { ICompanionDayDigest } from '@/common/adapter/ipcBridge';
import type { TMessage } from '@/common/chat/chatLib';
import { parseCompanionId, parseConversationId, parseMessageId } from '@/common/types/ids';
import DayIndexRail from './DayIndexRail';
import DayReader from './DayReader';
import { dayKeyOf, formatDayKey, messageCursorOf, toHistoryEntry } from './historyFormat';
import type { HistoryDay } from './useChatHistory';

const CONVERSATION_ID = parseConversationId('0198f6b1-0ef0-7000-8000-0000000000aa');
const COMPANION_ID = parseCompanionId('0198f6b1-0ef0-7000-8000-000000000001');
const WINDOW_ID = '0198f6b1-0ef0-7000-8000-0000000000bb';

/** 2026-08-04 09:30 local. */
const at = (hour: number, minute = 0): number => new Date(2026, 7, 4, hour, minute, 0, 0).getTime();

const textMessage = (id: string, position: 'left' | 'right', content: string, when: number): TMessage =>
  ({
    id,
    message_id: parseMessageId(`0198f6b1-0ef0-7000-8000-0000000001${id}`),
    conversation_id: CONVERSATION_ID,
    type: 'text',
    content: { content },
    position,
    hidden: false,
    created_at: when,
  }) as TMessage;

describe('history projection', () => {
  test('projects user and companion text onto reader lines', () => {
    const user = toHistoryEntry(textMessage('01', 'right', '今天状态怎么样？', at(9, 30)));
    const companion = toHistoryEntry(textMessage('02', 'left', '一切正常。', at(9, 31)));
    expect(user?.role).toBe('user');
    expect(user?.kind).toBe('text');
    expect(companion?.role).toBe('companion');
    expect(user?.day).toBe(dayKeyOf(at(9, 30)));
    expect(formatDayKey(user!.day)).toBe('2026-08-04');
  });

  test('drops hidden rows, empty text and non-readable types', () => {
    const hidden = { ...textMessage('03', 'left', 'x', at(10)), hidden: true } as TMessage;
    expect(toHistoryEntry(hidden)).toBeNull();
    expect(toHistoryEntry(textMessage('04', 'left', '   ', at(10)))).toBeNull();
    expect(
      toHistoryEntry({
        id: '05',
        conversation_id: CONVERSATION_ID,
        type: 'available_commands',
        content: { commands: [] },
        hidden: false,
        created_at: at(10),
      } as unknown as TMessage)
    ).toBeNull();
  });

  test('summarises a tool call as a single muted line', () => {
    const entry = toHistoryEntry({
      id: '06',
      conversation_id: CONVERSATION_ID,
      type: 'tool_call',
      content: { call_id: 'c1', name: 'read_file' },
      hidden: false,
      created_at: at(11),
    } as unknown as TMessage);
    expect(entry?.kind).toBe('tool');
    expect(entry?.text).toBe('read_file');
  });

  test('builds the keyset cursor the messages endpoint expects', () => {
    const message = textMessage('07', 'right', 'hi', at(12));
    expect(messageCursorOf(message)).toBe(`${at(12)}:${message.message_id}`);
  });
});

const digest: ICompanionDayDigest = {
  session_window_id: WINDOW_ID as ICompanionDayDigest['session_window_id'],
  companion_id: COMPANION_ID,
  conversation_id: CONVERSATION_ID,
  session_day: dayKeyOf(at(9)),
  started_at: at(9),
  last_activity_at: at(12),
  closed_at: at(12),
  status: 'closed',
  message_count: 2,
  boundary_ts: at(12),
  digest: '聊了部署计划，决定先做灰度。',
  highlights: JSON.stringify({ topics: ['部署'], mood: '专注' }),
  token_estimate: 120,
};

const day: HistoryDay = {
  day: dayKeyOf(at(9)),
  entries: [
    toHistoryEntry(textMessage('11', 'right', '今天状态怎么样？', at(9, 30)))!,
    toHistoryEntry(textMessage('12', 'left', '一切正常。', at(9, 31)))!,
  ],
  digests: [digest],
};

describe('history reader rendering', () => {
  test('renders the day digest above the day messages', () => {
    const html = renderToStaticMarkup(React.createElement(DayReader, { day, companionName: '小南' }));
    expect(html.includes('2026-08-04')).toBe(true);
    expect(html.includes('聊了部署计划，决定先做灰度。')).toBe(true);
    expect(html.includes('今天状态怎么样？')).toBe(true);
    expect(html.includes('一切正常。')).toBe(true);
    expect(html.includes('小南')).toBe(true);
    expect(html.indexOf('聊了部署计划')).toBeLessThan(html.indexOf('今天状态怎么样？'));
    // Sticky day header, and no scrollport wrapper that would neutralise it.
    expect(html.includes('sticky top-0')).toBe(true);
  });

  test('day rail marks digest days and offers an explicit 加载更早', () => {
    const html = renderToStaticMarkup(
      React.createElement(DayIndexRail, {
        days: [day],
        selectedDay: day.day,
        onSelect: () => {},
        hasMore: true,
        loadingMore: false,
        onLoadMore: () => {},
        entryCount: 2,
        oldestDay: day.day,
      })
    );
    expect(html.includes('加载更早')).toBe(true);
    expect(html.includes('!bg-primary-1 !text-primary-6')).toBe(true);
    expect(html.includes('这一天有日记')).toBe(true);
    expect(html.includes('role="button"')).toBe(true);
    // The boundary is named, not left as a vague "这一天". Assert only the stable
    // prose: whether `t()` interpolates the {{day}} placeholder depends on whether
    // some earlier test in the suite initialised i18next, so asserting the raw
    // placeholder would make this test order-dependent.
    expect(html.includes('日期索引只到')).toBe(true);
  });

  test('a failed 加载更早 says so instead of silently stopping', () => {
    const html = renderToStaticMarkup(
      React.createElement(DayIndexRail, {
        days: [day],
        selectedDay: day.day,
        onSelect: () => {},
        hasMore: true,
        loadingMore: false,
        onLoadMore: () => {},
        entryCount: 2,
        oldestDay: day.day,
        loadMoreFailed: true,
      })
    );
    expect(html.includes('加载失败，点此重试')).toBe(true);
  });

  test('the oldest loaded day admits it is only partly loaded', () => {
    const complete = renderToStaticMarkup(React.createElement(DayReader, { day, companionName: '小南' }));
    expect(complete.includes('只加载了后半段')).toBe(false);
    const partial = renderToStaticMarkup(
      React.createElement(DayReader, { day, companionName: '小南', partial: true })
    );
    expect(partial.includes('只加载了后半段')).toBe(true);
  });
});
