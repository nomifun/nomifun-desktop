/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import { parseChannelPluginId, parseCsAgentId } from '@/common/types/ids';
import { csBotBindingState, findNewlyCreatedCsBot, selectCsChannelBots } from './csChannelBots';

const BOT_A = parseChannelPluginId('0190f5fe-7c00-7a00-8000-000000000021');
const BOT_B = parseChannelPluginId('0190f5fe-7c00-7a00-8000-000000000022');
const BOT_C = parseChannelPluginId('0190f5fe-7c00-7a00-8000-000000000023');
const AGENT_ME = parseCsAgentId('0190f5fe-7c00-7a00-8000-000000000031');
const AGENT_OTHER = parseCsAgentId('0190f5fe-7c00-7a00-8000-000000000032');

const row = (patch: Partial<IChannelPluginStatus>): IChannelPluginStatus => ({
  plugin_id: BOT_A,
  type: 'telegram',
  name: 'Bot',
  enabled: true,
  connected: true,
  activeUsers: 0,
  hasToken: true,
  owner_domain: 'customer_service',
  ...patch,
});

describe('selectCsChannelBots', () => {
  test('keeps only customer-service domain rows (self-closed pool, never companion bots)', () => {
    const statuses = [
      row({ plugin_id: BOT_A, owner_domain: 'customer_service' }),
      row({ plugin_id: BOT_B, owner_domain: 'companion' }),
      row({ plugin_id: BOT_C, owner_domain: 'customer_service' }),
    ];
    expect(selectCsChannelBots(statuses).map((s) => s.plugin_id)).toEqual([BOT_A, BOT_C]);
  });
});

describe('csBotBindingState', () => {
  const ownerByBot = new Map([
    [BOT_A, AGENT_ME],
    [BOT_B, AGENT_OTHER],
  ]);

  test('distinguishes bound-to-this / bound-to-other / unbound', () => {
    expect(csBotBindingState(BOT_A, AGENT_ME, ownerByBot)).toEqual({ kind: 'boundToThis' });
    expect(csBotBindingState(BOT_B, AGENT_ME, ownerByBot)).toEqual({
      kind: 'boundToOther',
      csAgentId: AGENT_OTHER,
    });
    expect(csBotBindingState(BOT_C, AGENT_ME, ownerByBot)).toEqual({ kind: 'unbound' });
  });
});

describe('findNewlyCreatedCsBot', () => {
  test('finds only a cs-domain bot of the target platform that appeared after the snapshot', () => {
    const known = new Set([BOT_A]);
    const statuses = [
      row({ plugin_id: BOT_A }), // already known
      row({ plugin_id: BOT_B, owner_domain: 'companion' }), // wrong domain
      row({ plugin_id: BOT_C, type: 'lark' }), // wrong platform
    ];
    expect(findNewlyCreatedCsBot(statuses, 'telegram', known)).toBeNull();

    const created = row({ plugin_id: BOT_C, type: 'telegram' });
    expect(findNewlyCreatedCsBot([...statuses, created], 'telegram', known)).toBe(created);
  });
});
