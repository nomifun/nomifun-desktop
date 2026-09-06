/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { parseAgentId, parseAgentPresetId } from '@/common/types/ids';
import type { AgentPresetSummary } from '@/common/types/agentPlatform';
import type { AgentMetadata } from '@/renderer/utils/model/agentTypes';
import { CronAgentOptionIdentity, CronPresetOptionIdentity } from './CronAgentOptionIdentity';

const preset: AgentPresetSummary = {
  preset_id: parseAgentPresetId('0190f5fe-7c00-7a00-8000-000000000031'),
  source: 'user',
  display_name: 'Bug troubleshooting',
  bound_target_count: 0,
};

describe('scheduled task Agent option identity rendering', () => {
  test('renders the canonical saved Agent name without legacy avatar fields', () => {
    const html = renderToStaticMarkup(<CronPresetOptionIdentity preset={preset} />);
    expect(html.includes('Bug troubleshooting')).toBe(true);
    expect(html.includes('src=')).toBe(false);
  });

  test('renders supported Emoji as decoration while the Agent name remains the label', () => {
    const agent = {
      agent_id: parseAgentId('0190f5fe-7c00-7a00-8000-000000000032'),
      name: 'Custom reviewer',
      icon: '👋🏽',
      agent_type: 'nomi',
      agent_source: 'custom',
      enabled: true,
      available: true,
    } as AgentMetadata;
    const html = renderToStaticMarkup(<CronAgentOptionIdentity agent={agent} language='en-US' />);
    expect(html.includes('👋🏽')).toBe(true);
    expect(html.includes('Custom reviewer')).toBe(true);
    expect(html.includes('src="👋🏽"')).toBe(false);
  });

  test('places an unavailable status after the configured name', () => {
    const html = renderToStaticMarkup(
      <CronPresetOptionIdentity preset={preset} statusLabel='Unavailable' />
    );
    expect(html.indexOf('Bug troubleshooting')).toBeLessThan(html.indexOf('Unavailable'));
    expect(html.includes('flex-col')).toBe(true);
  });

  test('keeps the closed Select value compact while retaining status context', () => {
    const html = renderToStaticMarkup(
      <CronPresetOptionIdentity preset={preset} statusLabel='Unavailable' compact />
    );
    expect(html.includes('flex-col')).toBe(false);
    expect(html.includes('Bug troubleshooting')).toBe(true);
    expect(html.includes('Unavailable')).toBe(true);
    expect(html.includes('position:absolute')).toBe(true);
    expect(html.includes('text-12px')).toBe(false);
  });
});
