/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const bridgeSource = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');
const typeSource = readFileSync(
  new URL('../../renderer/utils/model/agentTypes.ts', import.meta.url),
  'utf8'
);

describe('agent metadata wire ID contract', () => {
  test('uses agent_id without a generic id compatibility path', () => {
    expect(typeSource.includes('agent_id: AgentId;')).toBe(true);
    expect(typeSource.includes('\n  id: string;')).toBe(false);
    expect(bridgeSource.includes('AgentMetadata legacy field "id" is not accepted')).toBe(true);
    expect(bridgeSource.includes('agent_id: parseAgentId(value.agent_id)')).toBe(true);
    expect(bridgeSource.includes("value.agent_source === 'custom' || value.agent_source === 'extension'")).toBe(false);
    // Only three `/api/agents*` routes survived the engine collapse: the list,
    // the availability refresh, and the model-provider probe. The custom-agent
    // CRUD, the per-row enabled toggle and the engine health-check went with
    // the engines that owned them, so the bridge must ship no client for them —
    // a surviving client would only ever produce a 404.
    expect(bridgeSource.includes('/api/agents/custom/')).toBe(false);
    expect(bridgeSource.includes('/api/agents/health-check')).toBe(false);
    expect(bridgeSource.includes('/enabled')).toBe(false);
    for (const survivor of ['/api/agents', '/api/agents/refresh', '/api/agents/provider-health-check']) {
      expect(bridgeSource.includes(`'${survivor}'`)).toBe(true);
    }
  });
});
