/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

/**
 * 渠道所有权分域（owner_domain）结构契约：
 * - wire 层按 Interfaces 契约映射 `owner_domain`（缺省 companion）；
 * - 所有平台表单经由唯一构造点 buildEnablePluginRequest 组装 enable 请求，
 *   不得再手搓 companion_id 展开（客服域创建必须打 owner_domain 且互斥于绑宠）；
 * - 伙伴侧「远程连接」只见 companion 域。
 */

const channelsDir = new URL('../settings/SettingsModal/contents/channels/', import.meta.url);
const FORM_FILES = [
  'TelegramConfigForm.tsx',
  'DiscordConfigForm.tsx',
  'SlackConfigForm.tsx',
  'MatrixConfigForm.tsx',
  'MattermostConfigForm.tsx',
  'TwitchConfigForm.tsx',
  'NostrConfigForm.tsx',
  'QQBotConfigForm.tsx',
  'LarkConfigForm.tsx',
  'DingTalkConfigForm.tsx',
  'WeixinConfigForm.tsx',
  'WecomConfigForm.tsx',
];

const bridgeSource = readFileSync(
  new URL('../../../common/adapter/ipcBridge.ts', import.meta.url),
  'utf8'
);
const bodySource = readFileSync(new URL('./PlatformConfigBody.tsx', import.meta.url), 'utf8');
const remoteConnectSource = readFileSync(
  new URL('../../pages/nomi/workspace/tabs/RemoteTab/RemoteConnectSection.tsx', import.meta.url),
  'utf8'
);

describe('channel owner_domain wire contract', () => {
  test('plugin status maps owner_domain and defaults missing values to companion', () => {
    expect(
      bridgeSource.includes(
        "owner_domain: raw.owner_domain === 'customer_service' ? 'customer_service' : 'companion'"
      )
    ).toBe(true);
  });

  test('enablePlugin accepts an optional owner_domain on create', () => {
    expect(bridgeSource.includes('owner_domain?: ChannelOwnerDomain;')).toBe(true);
  });
});

describe('platform forms route enable payloads through the single builder', () => {
  for (const file of FORM_FILES) {
    test(`${file} uses buildEnablePluginRequest and never hand-rolls companion_id`, () => {
      const source = readFileSync(new URL(file, channelsDir), 'utf8');
      expect(source.includes('buildEnablePluginRequest(')).toBe(true);
      expect(source.includes('companion_id: channelTarget')).toBe(false);
    });
  }

  test('PlatformConfigBody uses the builder and forwards the owner domain to resolution', () => {
    expect(bodySource.includes('buildEnablePluginRequest(platform, channelTarget, config)')).toBe(true);
    expect(bodySource.includes('ownerDomain: channelTarget?.ownerDomain')).toBe(true);
    expect(bodySource.includes('companion_id: channelTarget')).toBe(false);
  });
});

describe('companion-side channel surfaces are companion-domain scoped', () => {
  test('RemoteConnectSection filters plugin statuses to the companion domain', () => {
    expect(remoteConnectSource.includes("statusInOwnerDomain(plugin, 'companion')")).toBe(true);
  });
});
