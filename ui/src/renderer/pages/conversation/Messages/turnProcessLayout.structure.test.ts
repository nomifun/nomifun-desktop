/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const cssSource = readFileSync(new URL('./messages.css', import.meta.url), 'utf8');
const disclosureSource = readFileSync(new URL('./components/TurnProcessDisclosure.tsx', import.meta.url), 'utf8');
const processTraceSource = readFileSync(new URL('./components/ProcessTraceItem.tsx', import.meta.url), 'utf8');
const messageListSource = readFileSync(new URL('./MessageList.tsx', import.meta.url), 'utf8');
const modelSource = readFileSync(new URL('./turnDisclosureModel.ts', import.meta.url), 'utf8');
type MessagesLocale = Record<string, unknown> & {
  turnDuration: string;
  turnDurationUnknown: string;
  turnProcess: { expand: string; collapse: string };
};
const zhMessages = JSON.parse(
  readFileSync(new URL('../../../services/i18n/locales/zh-CN/messages.json', import.meta.url), 'utf8')
) as MessagesLocale;
const enMessages = JSON.parse(
  readFileSync(new URL('../../../services/i18n/locales/en-US/messages.json', import.meta.url), 'utf8')
) as MessagesLocale;

const cssRuleFor = (selector: string) => {
  const start = cssSource.indexOf(selector);
  if (start < 0) return '';
  const open = cssSource.indexOf('{', start);
  const close = cssSource.indexOf('}', open);
  return open >= 0 && close > open ? cssSource.slice(open + 1, close) : '';
};

describe('turn process disclosure result-first layout', () => {
  test('shows only duration and disclosure affordance in the turn header', () => {
    expect(disclosureSource.includes('messages.turnDuration')).toBe(true);
    expect(disclosureSource.includes('messages.turnProcessed')).toBe(false);
    expect(disclosureSource.includes('messages.turnCanceled')).toBe(false);
    expect(disclosureSource.includes('messages.turnFailed')).toBe(false);
    expect(disclosureSource.includes('messages.turnSuccess')).toBe(false);
    expect(zhMessages.turnDuration).toBe('用时 {{duration}}');
    expect(zhMessages.turnDurationUnknown).toBe('用时 --');
    expect(enMessages.turnDuration).toBe('Took {{duration}}');
    expect(enMessages.turnDurationUnknown).toBe('Time --');
    expect(zhMessages.turnProcess).toEqual({ expand: '展开思考过程', collapse: '收起思考过程' });
  });

  test('keeps the duration live while the current turn is running', () => {
    expect(disclosureSource.includes('if (!item.running) return;')).toBe(true);
    expect(disclosureSource.includes('window.setInterval')).toBe(true);
    expect(disclosureSource.includes('const durationEndAt = item.running ? now : item.endAt;')).toBe(true);
    expect(disclosureSource.includes("item.running && 'turn-process-disclosure--live'")).toBe(true);
  });

  test('opens running thought by default and collapses when the turn settles', () => {
    expect(disclosureSource.includes('hasProcessItems && !defaultCollapsed')).toBe(true);
    expect(modelSource.includes("defaultCollapsed: state !== 'running'")).toBe(true);
    expect(disclosureSource.includes('shouldResetTurnProcessDisclosureExpansion')).toBe(true);
  });

  test('removes the legacy live-step and expand-all paths', () => {
    expect(messageListSource.includes('turn-live-step')).toBe(false);
    expect(messageListSource.includes('planTurnLiveStep')).toBe(false);
    expect(disclosureSource.includes('getProcessItemCanExpandAll')).toBe(false);
    expect(disclosureSource.includes('expandAllProcessItemKeys')).toBe(false);
    expect(cssSource.includes('.turn-live-step')).toBe(false);
    expect(cssSource.includes('.turn-process-disclosure__header-actions')).toBe(false);
  });

  test('keeps durable turn summaries as timing metadata rather than visible rows', () => {
    expect(messageListSource.includes("item.content.turn_summary ? 'metadata' : 'process'")).toBe(true);
    expect(processTraceSource.includes('if (item.content.turn_summary) return null;')).toBe(true);
    expect(modelSource.includes("entry.role === 'metadata'")).toBe(true);
  });

  test('renders thinking directly as paragraphs and removes private replay placeholders', () => {
    expect(processTraceSource.includes('turn-process-trace--thinking')).toBe(true);
    expect(processTraceSource.includes('Private reasoning omitted')).toBe(true);
    expect(processTraceSource.includes('<MessageThinking')).toBe(false);
    expect(messageListSource.includes('isHiddenProcessItem')).toBe(true);
    expect(messageListSource.includes("item.type !== 'thinking' && item.type !== 'text'")).toBe(true);
  });

  test('uses compact process rhythm and muted receipt rows', () => {
    expect(cssRuleFor('.turn-process-disclosure__body').includes('gap: 8px')).toBe(true);
    expect(cssRuleFor('.turn-process-disclosure__body').includes('padding: 10px 0 12px')).toBe(true);
    expect(
      cssRuleFor('.turn-process-disclosure__body .turn-process-trace__row').includes(
        'color: var(--color-text-3'
      )
    ).toBe(true);
    expect(
      cssRuleFor('.turn-process-disclosure__body .turn-process-trace__paragraph').includes(
        'color: var(--color-text-2'
      )
    ).toBe(true);
  });

  test('shimmers only live duration and the current thinking line', () => {
    expect(cssSource.includes('.turn-process-disclosure--live .turn-process-disclosure__label')).toBe(true);
    expect(cssSource.includes('@keyframes turn-process-shimmer')).toBe(true);
    expect(cssSource.includes('@keyframes turn-process-current-fade')).toBe(true);
    expect(cssSource.includes('.turn-process-trace__thinking-last-line')).toBe(true);
    expect(cssSource.includes('.turn-process-disclosure__item--current .turn-process-trace__row--running')).toBe(false);
    expect(cssSource.includes('.turn-process-trace__row--current-activity .turn-process-trace__text')).toBe(true);
    expect(cssSource.includes('prefers-reduced-motion: reduce')).toBe(true);
  });

  test('keeps content-kind hooks for current-row targeting', () => {
    expect(disclosureSource.includes('getProcessItemLayoutKind')).toBe(true);
    expect(disclosureSource.includes('turn-process-disclosure__item--')).toBe(true);
    expect(disclosureSource.includes("'turn-process-disclosure__item--current'")).toBe(true);
    expect(messageListSource.includes('getProcessItemLayoutKind={getProcessItemLayoutKind}')).toBe(true);
  });

  test('keeps the shared conversation typography contract', () => {
    const itemRule = cssRuleFor('.message-item');
    expect(itemRule.includes('--conversation-message-font-size: 14px')).toBe(true);
    expect(itemRule.includes('--conversation-message-line-height: 22px')).toBe(true);
  });
});
