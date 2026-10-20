/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  isProcessTextEchoOfFinal,
  selectJournalProcessItems,
  stripInternalToolCallPayload,
} from './processTraceDisplayModel';

describe('process trace display model', () => {
  test('hides completed private activity without losing public progress or tool order', () => {
    const items = [
      { kind: 'private', running: false, text: 'thought 1' },
      { kind: 'private', running: false, text: 'thought 2' },
      { kind: 'public', running: false, text: 'I found the cause.' },
      { kind: 'tool', running: false, text: 'Edited two files' },
      { kind: 'private', running: false, text: 'thought 3' },
      { kind: 'public', running: false, text: 'I am verifying the fix.' },
    ];
    const select = (running: boolean) => selectJournalProcessItems(items, {
      running,
      isPrivateActivity: (item) => item.kind === 'private',
      isRunning: (item) => item.running,
    });

    expect(select(false).map((item) => item.text)).toEqual([
      'I found the cause.', 'Edited two files', 'I am verifying the fix.',
    ]);
    expect(select(true)).toEqual(select(false));
  });

  test('keeps one live private activity only while it is the current tail', () => {
    const items = [
      { kind: 'tool', running: false },
      { kind: 'private', running: true },
    ];
    const options = {
      running: true,
      isPrivateActivity: (item: (typeof items)[number]) => item.kind === 'private',
      isRunning: (item: (typeof items)[number]) => item.running,
    };
    expect(selectJournalProcessItems(items, options)).toEqual(items);
    expect(selectJournalProcessItems([...items, { kind: 'tool', running: false }], options))
      .toEqual([items[0], { kind: 'tool', running: false }]);
  });

  test('removes a malformed model tool invocation without exposing embedded HTML', () => {
    const payload = [
      'I will make the page.',
      '<tool_call>',
      '',
      '<function=write_file>',
      '<parameter=path>',
      'index.html',
      '</parameter>',
      '<parameter=content>',
      '<!DOCTYPE html>',
      '<style>body { color: red; }</style>',
      '</parameter>',
      '</tool_call>',
      'I will check the result.',
    ].join('\n');

    expect(stripInternalToolCallPayload(payload)).toBe(
      'I will make the page.\n\nI will check the result.'
    );
    expect(stripInternalToolCallPayload(payload.slice(payload.indexOf('<tool_call>')))).toBe('I will check the result.');
    expect(stripInternalToolCallPayload('<tool_call>\n<function=write_file>\n<parameter=content>\n<!DOCTYPE html>'))
      .toBe('');
    expect(stripInternalToolCallPayload(
      'I will continue the page. <tool_call>\n<function=write_file>\n<parameter=content>\n<!DOCTYPE html>'
    )).toBe('I will continue the page.');
  });

  test('preserves a fenced example and unrelated XML-shaped prose', () => {
    const example = '```xml\n<tool_call>\n<function=write_file>\n</tool_call>\n```';
    expect(stripInternalToolCallPayload(example)).toBe(example);
    expect(stripInternalToolCallPayload('The literal <tool_call> tag is documented here.'))
      .toBe('The literal <tool_call> tag is documented here.');
  });

  test('suppresses only an intermediate answer that repeats the final answer', () => {
    const finalAnswer = 'The game is ready.\n\n- Use arrow keys\n- Press Space to pause';
    expect(isProcessTextEchoOfFinal(
      'The game is ready.\n\n- Use arrow keys\n\n- Press Space to pause',
      finalAnswer
    )).toBe(true);
    expect(isProcessTextEchoOfFinal('I am writing the game now.', finalAnswer)).toBe(false);
    expect(isProcessTextEchoOfFinal(finalAnswer)).toBe(false);
  });

  test('keeps the latest live progress and removes its final-answer echo only after completion', () => {
    const items = [
      { kind: 'text', text: 'I found the cause.' },
      { kind: 'tool', text: 'Read a file' },
      { kind: 'text', text: 'The fix is ready.' },
    ];
    const options = {
      isPrivateActivity: () => false,
      isRunning: () => false,
      textOf: (item: (typeof items)[number]) => item.kind === 'text' ? item.text : undefined,
      finalText: 'The fix is ready.',
    };
    expect(selectJournalProcessItems(items, { ...options, running: true })).toEqual(items);
    expect(selectJournalProcessItems(items, { ...options, running: false })).toEqual(items.slice(0, 2));
  });

});
