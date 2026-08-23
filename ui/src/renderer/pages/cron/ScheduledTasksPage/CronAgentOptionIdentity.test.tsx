/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { parseAgentId } from '@/common/types/ids';
import { parsePresetReference, type Preset } from '@/common/types/agent/presetTypes';
import type { AgentMetadata } from '@/renderer/utils/model/agentTypes';
import { CronAgentOptionIdentity, CronPresetOptionIdentity } from './CronAgentOptionIdentity';

const cdnAvatar =
  'https://cloudcache.tencent-cloud.com/qcloud/tea/app/skillhub/assets/source/ai-buddy-decouple/expert-profiles/tech-bug-troubleshooting.v20260625.avif';

const preset = {
  preset_id: parsePresetReference('0190f5fe-7c00-7a00-8000-000000000031'),
  name: 'Bug troubleshooting',
  name_i18n: { 'zh-CN': 'Bug 排查' },
  avatar: cdnAvatar,
} as unknown as Preset;

describe('scheduled task Agent option identity rendering', () => {
  test('renders the reported CDN value only as an image source and keeps the configured name visible', () => {
    const html = renderToStaticMarkup(<CronPresetOptionIdentity preset={preset} language='zh-CN' />);
    expect(html.includes(`src="${cdnAvatar}"`)).toBe(true);
    expect(html.includes('Bug 排查')).toBe(true);
    expect(html.includes(`>${cdnAvatar}<`)).toBe(false);
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

  test('places an unavailable status on a second line so it cannot displace the configured name', () => {
    const html = renderToStaticMarkup(
      <CronPresetOptionIdentity preset={preset} language='zh-CN' statusLabel='未启用定时任务' />
    );
    expect(html.indexOf('Bug 排查')).toBeLessThan(html.indexOf('未启用定时任务'));
    expect(html.includes('flex-col')).toBe(true);
  });

  test('keeps the closed Select value compact while retaining status context', () => {
    const html = renderToStaticMarkup(
      <CronPresetOptionIdentity
        preset={preset}
        language='zh-CN'
        statusLabel='未启用定时任务'
        compact
      />
    );
    expect(html.includes('flex-col')).toBe(false);
    expect(html.includes('title="Bug 排查 · 未启用定时任务"')).toBe(true);
    expect(html.includes('position:absolute')).toBe(true);
    expect(html.includes('text-12px')).toBe(false);
  });
});
