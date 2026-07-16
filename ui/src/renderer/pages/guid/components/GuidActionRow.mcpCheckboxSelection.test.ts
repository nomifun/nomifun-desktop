/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const actionRowSource = readFileSync(new URL('./GuidActionRow.tsx', import.meta.url), 'utf8');
const controlCss = readFileSync(new URL('../../../styles/theme-control-contract.css', import.meta.url), 'utf8');

describe('GuidActionRow MCP checkbox selection treatment', () => {
  test('applies the enhanced theme-aware checkbox treatment to MCP server choices', () => {
    expect(actionRowSource.includes("className='guid-mcp-selection-checkbox'")).toBe(true);
    expect(controlCss.includes('.arco-checkbox-checked .arco-checkbox-mask')).toBe(true);
    expect(controlCss.includes('.arco-checkbox-mask-icon')).toBe(true);
  });
});
