/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('TerminalCreatePage extended capabilities', () => {
  test('wires smart decision as a create-time draft capability', () => {
    const createPageSource = readSource(new URL('./TerminalCreatePage.tsx', import.meta.url));
    const panelSource = readSource(new URL('./ExtendedCapabilitiesPanel.tsx', import.meta.url));

    expect(createPageSource.includes('defaultIdmmConfig')).toBe(true);
    expect(createPageSource.includes('const [idmm, setIdmm]')).toBe(true);
    expect(createPageSource.includes('ipcBridge.idmm.set.invoke')).toBe(true);
    expect(createPageSource.includes("kind: 'terminal'")).toBe(true);
    expect(createPageSource.includes('target_id: session.terminal_id')).toBe(true);

    expect(panelSource.includes('IdmmControl')).toBe(true);
    // The draft declares its kind: a terminal has no model of its own to lend the
    // model tier (its agent CLI owns the model), so a terminal watch must name a
    // bypass model itself. Without this the control would offer a one-click
    // enable that the backend then rejects with a 400.
    expect(panelSource.includes("draft={{ value: idmm, onChange: onIdmmChange, kind: 'terminal' }}")).toBe(true);
  });
});
