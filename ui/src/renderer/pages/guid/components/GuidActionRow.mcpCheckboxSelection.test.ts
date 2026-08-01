/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const actionRowSource = readFileSync(new URL('./GuidActionRow.tsx', import.meta.url), 'utf8');
const modelSelectorSource = readFileSync(new URL('./GuidModelSelector.tsx', import.meta.url), 'utf8');
const guidCss = readFileSync(new URL('../index.module.css', import.meta.url), 'utf8');
const controlCss = readFileSync(new URL('../../../styles/theme-control-contract.css', import.meta.url), 'utf8');

describe('GuidActionRow MCP checkbox selection treatment', () => {
  test('applies the enhanced theme-aware checkbox treatment to MCP server choices', () => {
    expect(actionRowSource.includes("className='guid-mcp-selection-checkbox'")).toBe(true);
    expect(controlCss.includes('.arco-checkbox-checked .arco-checkbox-mask')).toBe(true);
    expect(controlCss.includes('.arco-checkbox-mask-icon')).toBe(true);
  });

  test('collapses model configuration labels to hover-expand icons in a narrow action slot', () => {
    expect(actionRowSource.includes('styles.actionConfigGroupResponsive')).toBe(true);
    expect(actionRowSource.includes('styles.actionSubmitResponsive')).toBe(true);
    expect(modelSelectorSource.includes('sendbox-responsive-label')).toBe(true);
    expect(modelSelectorSource.includes('sendbox-responsive-chevron')).toBe(true);
    expect(modelSelectorSource.includes('<Tooltip')).toBe(false);
    expect(guidCss.includes('container-name: guid-action-config')).toBe(true);
    expect(guidCss.includes('@container guid-action-config (max-width: 440px)')).toBe(true);
    expect(guidCss.includes(':global(.guid-config-btn:hover)')).toBe(true);
    expect(guidCss.includes('@media (hover: hover) and (pointer: fine)')).toBe(true);
  });
});
