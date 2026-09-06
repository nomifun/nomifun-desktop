/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = () => readFileSync(new URL('./LocalAgents.tsx', import.meta.url), 'utf8');
const cardSource = () => readFileSync(new URL('./AgentCard.tsx', import.meta.url), 'utf8');

describe('LocalAgents execution-engine management boundary', () => {
  test('keeps a manual re-scan control on the local agents surface', () => {
    const text = source();

    expect(text.includes('refreshCustomAgents')).toBe(true);
    expect(text.includes("data-testid='btn-refresh-local-agents'")).toBe(true);
    expect(text.includes('handleRefreshDetection')).toBe(true);
    expect(text.includes('settings.agentManagement.refreshDetection')).toBe(true);
  });

  test('does not launch Guid from execution-engine metadata', () => {
    const localAgents = source();
    const agentCard = cardSource();

    expect(localAgents.includes('useNavigate')).toBe(false);
    expect(localAgents.includes("navigate('/guid'")).toBe(false);
    expect(localAgents.includes('selectedAgentKey')).toBe(false);
    expect(localAgents.includes('getAgentKey')).toBe(false);
    expect(localAgents.includes('onGoToChat')).toBe(false);
    expect(agentCard.includes('onGoToChat')).toBe(false);
    expect(agentCard.includes('settings.agentManagement.goToChat')).toBe(false);
  });
});
