/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const messageListSource = readFileSync(new URL('./MessageList.tsx', import.meta.url), 'utf8');
const cssSource = readFileSync(new URL('./messages.css', import.meta.url), 'utf8');
const zhMessages = JSON.parse(
  readFileSync(new URL('../../../services/i18n/locales/zh-CN/messages.json', import.meta.url), 'utf8')
) as Record<string, Record<string, string> | string>;
const enMessages = JSON.parse(
  readFileSync(new URL('../../../services/i18n/locales/en-US/messages.json', import.meta.url), 'utf8')
) as Record<string, Record<string, string> | string>;

describe('turn live step strip', () => {
  test('appends the live step to the display list on both return paths', () => {
    expect(messageListSource.includes("import { planTurnLiveStep } from './turnLiveStepModel'")).toBe(true);
    expect(messageListSource.includes('const liveStepForDisclosures = buildTurnLiveStep(disclosureItems)')).toBe(true);
    expect(messageListSource.includes('const liveStep = buildTurnLiveStep(withDeliverables)')).toBe(true);
    expect(messageListSource.includes("data-testid='turn-live-step'")).toBe(true);
  });

  test('renders through the existing receipt row without detail expansion', () => {
    expect(messageListSource.includes("type: 'turn_live_step'")).toBe(true);
    expect(messageListSource.includes('hasDetail: false')).toBe(true);
  });

  test('breathes gently and respects reduced motion', () => {
    expect(cssSource.includes('@keyframes turn-live-step-breathing')).toBe(true);
    expect(cssSource.includes('.turn-live-step .turn-process-receipt__label')).toBe(true);
    const reducedMotionBlocks = cssSource.split('@media (prefers-reduced-motion: reduce)').slice(1);
    expect(
      reducedMotionBlocks.some((block) =>
        block.slice(0, block.indexOf('}') + 200).includes('.turn-live-step .turn-process-receipt__label')
      )
    ).toBe(true);
  });

  test('ships bilingual live-step copy', () => {
    expect((zhMessages.turnLiveStep as Record<string, string>).analyzing).toBe('正在分析需求');
    expect((zhMessages.turnLiveStep as Record<string, string>).composing).toBe('正在整理回复');
    expect((enMessages.turnLiveStep as Record<string, string>).analyzing).toBeTruthy();
    expect((enMessages.turnLiveStep as Record<string, string>).composing).toBeTruthy();
  });
});
