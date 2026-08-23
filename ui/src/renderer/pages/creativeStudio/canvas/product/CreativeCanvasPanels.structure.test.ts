/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./CreativeCanvasPanels.tsx', import.meta.url), 'utf8');
const styles = readFileSync(new URL('./CreativeCanvasPanels.module.css', import.meta.url), 'utf8');

describe('Creative Canvas product panel boundaries', () => {
  test('is controlled presentation over the canonical canvas state', () => {
    expect(source.includes("import type { CanvasState } from '../core'")).toBe(true);
    expect(source.includes('state: CanvasState')).toBe(true);
    expect(source.includes("mode: 'replace' | 'toggle'")).toBe(true);
    expect(source.includes('onUndo(): void')).toBe(true);
    expect(source.includes('onRedo(): void')).toBe(true);
    expect(source.includes('useState')).toBe(false);
    expect(source.includes('useReducer')).toBe(false);
    expect(source.includes('useEffect')).toBe(false);
  });

  test('contains no persistence, model invocation, or fabricated agent surface', () => {
    for (const forbidden of [
      'localStorage',
      'sessionStorage',
      'fetch(',
      'httpRequest',
      'useCreativeProject',
      'CreativeStudioAgentPanel',
      'createNomiCreativeStudioAgentChatPort',
      'CreativeStudioAgentChatController',
      'onSend',
    ]) {
      expect(source.includes(forbidden)).toBe(false);
    }
    expect(source.includes("data-unavailable-kind={kind}")).toBe(true);
    expect(source.includes('creativeStudio.canvas.history.disclosure')).toBe(true);
    expect(source.includes('creativeStudio.canvas.unavailable.agentDescription')).toBe(true);
    expect(source.includes('creativeStudio.canvas.properties.editLabel')).toBe(true);
  });

  test('uses semantic theme tokens and IconPark rather than handwritten artwork', () => {
    expect(source.includes("from '@icon-park/react'")).toBe(true);
    expect(source.includes('<svg')).toBe(false);
    expect(styles.includes('var(--color-bg-2)')).toBe(true);
    expect(styles.includes('var(--color-text-1)')).toBe(true);
    expect(styles.includes('var(--color-border-2)')).toBe(true);
    expect(styles.includes('rgb(var(--primary-6))')).toBe(true);
  });
});
