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
  turnProcess: { expand: string; collapse: string; runningSummary?: string };
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

describe('turn process continuous work journal layout', () => {
  test('uses a quiet worked-duration disclosure header', () => {
    expect(disclosureSource.includes('messages.turnDuration')).toBe(true);
    expect(disclosureSource.includes('messages.turnProcessed')).toBe(false);
    expect(disclosureSource.includes('messages.turnCanceled')).toBe(false);
    expect(disclosureSource.includes('messages.turnFailed')).toBe(false);
    expect(disclosureSource.includes('messages.turnSuccess')).toBe(false);
    expect(zhMessages.turnDuration).toBe('已工作 {{duration}}');
    expect(zhMessages.turnDurationUnknown).toBe('已工作 --');
    expect(enMessages.turnDuration).toBe('Worked for {{duration}}');
    expect(enMessages.turnDurationUnknown).toBe('Worked for --');
    expect(zhMessages.turnProcess.expand).toBe('展开执行进度');
    expect(zhMessages.turnProcess.collapse).toBe('收起执行进度');
    expect(disclosureSource.includes('messages.turnProcess.runningSummary')).toBe(false);
  });

  test('keeps the duration live while the current turn is running', () => {
    expect(disclosureSource.includes('if (!item.running) return;')).toBe(true);
    expect(disclosureSource.includes('window.setInterval')).toBe(true);
    expect(disclosureSource.includes('const durationEndAt = item.running ? now : item.endAt;')).toBe(true);
    expect(disclosureSource.includes("item.running && 'turn-process-disclosure--live'")).toBe(true);
  });

  test('keeps the work journal open by default while allowing manual collapse', () => {
    expect(disclosureSource.includes('hasProcessItems && !defaultCollapsed')).toBe(true);
    expect(modelSource.includes('defaultCollapsed: false')).toBe(true);
    expect(modelSource.includes("defaultCollapsed: state !== 'running'")).toBe(false);
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

  test('renders public progress as prose while keeping private reasoning summarized', () => {
    expect(processTraceSource.includes("case 'text':")).toBe(true);
    expect(processTraceSource.includes("data-testid='process-narration'")).toBe(true);
    expect(processTraceSource.includes('<MarkdownView')).toBe(true);
    expect(processTraceSource.includes('Private reasoning omitted')).toBe(true);
    expect(processTraceSource.includes('<MessageThinking')).toBe(false);
    expect(messageListSource.includes('isHiddenProcessItem')).toBe(true);
    expect(messageListSource.includes("item.type !== 'thinking' && item.type !== 'text'")).toBe(true);
  });

  test('uses an unbounded document flow with compact muted receipt rows', () => {
    const bodyRule = cssRuleFor('.turn-process-disclosure__body');
    expect(bodyRule.includes('gap: 14px')).toBe(true);
    expect(bodyRule.includes('padding: 14px 0 4px')).toBe(true);
    expect(bodyRule.includes('overflow: visible')).toBe(true);
    expect(bodyRule.includes('max-height')).toBe(false);
    expect(bodyRule.includes('overflow-y: auto')).toBe(false);
    expect(bodyRule.includes('border-bottom')).toBe(false);
    expect(
      cssRuleFor('.turn-process-disclosure__body .turn-process-trace__row').includes(
        'color: var(--color-text-3'
      )
    ).toBe(true);
    expect(
      cssRuleFor('.turn-process-disclosure__body .turn-process-trace__paragraph').includes(
        'color: var(--color-text-1'
      )
    ).toBe(true);

    const receiptBodyRule = cssRuleFor('.turn-process-receipt__body');
    expect(receiptBodyRule.includes('max-height: min(360px, 42vh)')).toBe(true);
    expect(receiptBodyRule.includes('overflow-y: auto')).toBe(true);
  });

  test('lets Markdown control public-progress whitespace instead of inheriting pre-wrap', () => {
    const paragraphRule = cssRuleFor('\n.turn-process-trace__paragraph {');
    expect(paragraphRule.includes('white-space: normal')).toBe(true);
    expect(paragraphRule.includes('white-space: pre-wrap')).toBe(false);
  });

  test('animates only the current activity rather than the whole duration label', () => {
    expect(cssSource.includes('.turn-process-disclosure--live .turn-process-disclosure__label')).toBe(false);
    expect(cssSource.includes('@keyframes turn-process-shimmer')).toBe(false);
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
