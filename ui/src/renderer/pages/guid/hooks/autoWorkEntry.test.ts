/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { autoWorkStartDisabled, planGuidEntry } from './autoWorkEntry';

describe('planGuidEntry', () => {
  test('long Chinese input produces a valid single-line UTF-8 title while retaining a normal message send', () => {
    const input = `请检查中文项目 ${'验收🧪'.repeat(70)}\n第二项包含完整指令，不应成为会话标题。`;
    const plan = planGuidEntry(input, {enabled:false});
    expect(new TextEncoder().encode(plan.conversationName).length).toBeLessThanOrEqual(200);
    expect(plan.conversationName).not.toContain('\n');
    expect(plan.conversationName).not.toContain('\ufffd');
    expect(plan.sendInitialMessage).toBe(true);
    expect(input.split('\n')[0].startsWith(plan.conversationName)).toBe(true);
    expect(new TextDecoder('utf-8', {fatal:true}).decode(new TextEncoder().encode(plan.conversationName)))
      .toBe(plan.conversationName);
  });

  test('edge whitespace is removed from the derived title, including a trailing pasted newline', () => {
    expect(planGuidEntry('  执行这个命令\r\n', {enabled:false}).conversationName).toBe('执行这个命令');
    expect(planGuidEntry('先读取文件\n再运行测试', {enabled:false}).conversationName).toBe('先读取文件');
  });

  test('normal send (AutoWork off): sends the typed input as the first message', () => {
    const plan = planGuidEntry('do the thing', { enabled: false });
    expect(plan.autoWorkEntry).toBe(false);
    expect(plan.sendInitialMessage).toBe(true);
    expect(plan.conversationName).toBe('do the thing');
  });

  test('AutoWork entry (enabled + tag) does NOT send an initial message — avoids the running-turn race', () => {
    const plan = planGuidEntry('', { enabled: true, tag: 'release' });
    expect(plan.autoWorkEntry).toBe(true);
    expect(plan.sendInitialMessage).toBe(false);
    // No typed input → name falls back to the requirement tag.
    expect(plan.conversationName).toBe('release');
  });

  test('AutoWork entry keeps typed text as the conversation name but still does not send it', () => {
    const plan = planGuidEntry('  ship v2  ', { enabled: true, tag: 'release' });
    expect(plan.autoWorkEntry).toBe(true);
    expect(plan.sendInitialMessage).toBe(false);
    expect(plan.conversationName).toBe('ship v2');
  });

  test('enabled without a tag is not a valid AutoWork entry — falls back to normal send', () => {
    const plan = planGuidEntry('hello', { enabled: true });
    expect(plan.autoWorkEntry).toBe(false);
    expect(plan.sendInitialMessage).toBe(true);
    expect(plan.conversationName).toBe('hello');
  });
});

describe('autoWorkStartDisabled', () => {
  test('disabled while loading', () => {
    expect(autoWorkStartDisabled(true, { enabled: true, tag: 'release' })).toBe(true);
  });

  test('disabled when AutoWork is not a valid entry (off, or no tag)', () => {
    expect(autoWorkStartDisabled(false, { enabled: false })).toBe(true);
    expect(autoWorkStartDisabled(false, { enabled: true })).toBe(true);
  });

  test('enabled when AutoWork is on with a tag and not loading', () => {
    expect(autoWorkStartDisabled(false, { enabled: true, tag: 'release' })).toBe(false);
  });
});
