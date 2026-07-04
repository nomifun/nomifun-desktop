/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import enCommon from './locales/en-US/common.json';
import enConversation from './locales/en-US/conversation.json';
import enNomi from './locales/en-US/nomi.json';
import enPublicCompanion from './locales/en-US/publicCompanion.json';
import enSettings from './locales/en-US/settings.json';
import zhCommon from './locales/zh-CN/common.json';
import zhConversation from './locales/zh-CN/conversation.json';
import zhNomi from './locales/zh-CN/nomi.json';
import zhPublicCompanion from './locales/zh-CN/publicCompanion.json';
import zhSettings from './locales/zh-CN/settings.json';

type LocaleJson = Record<string, unknown>;

const CHANNEL_SETTINGS_KEYS = [
  'channels.weixinTitle',
  'channels.larkTitle',
  'channels.wecomTitle',
  'channels.dingtalkTitle',
  'channels.qqbotTitle',
  'channels.telegramTitle',
  'channels.discordTitle',
  'channels.slackTitle',
  'channels.matrixTitle',
  'channels.mattermostTitle',
  'channels.twitchTitle',
  'channels.nostrTitle',
  'assistant.tokenRequired',
  'assistant.tokenLocked',
  'assistant.testConnection',
  'assistant.connectionSuccess',
  'assistant.connectionFailed',
  'assistant.pluginEnabled',
  'assistant.pluginDisabled',
  'assistant.botToken',
  'assistant.botTokenDesc',
  'assistant.nextSteps',
  'assistant.step1',
  'assistant.step2',
  'assistant.step3',
  'assistant.step4',
  'assistant.pendingPairings',
  'assistant.noPendingPairings',
  'assistant.pairingCode',
  'assistant.expiresIn',
  'assistant.approve',
  'assistant.reject',
  'assistant.pairingApproved',
  'assistant.pairingRejected',
  'assistant.authorizedUsers',
  'assistant.noAuthorizedUsers',
  'assistant.platform',
  'assistant.authorizedAt',
  'assistant.revokeAccess',
  'assistant.userRevoked',
  'assistant.copyCode',
  'discord.botToken',
  'discord.botTokenDesc',
  'discord.tokenRequired',
  'discord.tokenLocked',
  'discord.connectionSuccess',
  'discord.connectionFailed',
  'discord.pluginEnabled',
  'discord.pluginDisabled',
  'discord.intentNote',
  'slack.botToken',
  'slack.appToken',
  'slack.tokensDesc',
  'slack.credentialsRequired',
  'slack.connectionSuccess',
  'slack.connectionFailed',
  'slack.pluginEnabled',
  'slack.pluginDisabled',
  'matrix.homeserver',
  'matrix.userId',
  'matrix.accessToken',
  'matrix.tokensDesc',
  'matrix.credentialsRequired',
  'matrix.connectionSuccess',
  'matrix.connectionFailed',
  'matrix.pluginEnabled',
  'matrix.pluginDisabled',
  'mattermost.serverUrl',
  'mattermost.botToken',
  'mattermost.tokensDesc',
  'mattermost.credentialsRequired',
  'mattermost.connectionSuccess',
  'mattermost.connectionFailed',
  'mattermost.pluginEnabled',
  'mattermost.pluginDisabled',
  'twitch.token',
  'twitch.channel',
  'twitch.tokensDesc',
  'twitch.credentialsRequired',
  'twitch.connectionSuccess',
  'twitch.connectionFailed',
  'twitch.pluginEnabled',
  'twitch.pluginDisabled',
  'nostr.privateKey',
  'nostr.relays',
  'nostr.tokensDesc',
  'nostr.credentialsRequired',
  'nostr.connectionSuccess',
  'nostr.connectionFailed',
  'nostr.pluginEnabled',
  'nostr.pluginDisabled',
  'qqbot.appId',
  'qqbot.clientSecret',
  'qqbot.tokensDesc',
  'qqbot.credentialsRequired',
  'qqbot.connectionSuccess',
  'qqbot.connectionFailed',
  'qqbot.pluginEnabled',
  'qqbot.pluginDisabled',
  'qqbot.intentNote',
  'lark.appId',
  'lark.devConsoleLink',
  'lark.appIdDescSuffix',
  'lark.appSecret',
  'lark.appSecretDescSuffix',
  'lark.encryptKey',
  'lark.encryptKeyDesc',
  'lark.verificationToken',
  'lark.verificationTokenDesc',
  'lark.optional',
  'lark.credentialsRequired',
  'lark.connectionSuccess',
  'lark.connectionFailed',
  'lark.pluginEnabled',
  'lark.pluginDisabled',
  'lark.enableFailed',
  'lark.testAndConnect',
  'lark.credentialsSaved',
  'lark.connectionStatus',
  'lark.statusConnected',
  'lark.statusError',
  'lark.statusConnecting',
  'lark.step1',
  'lark.step2',
  'lark.step3',
  'lark.step4',
  'lark.waitingConnection',
  'lark.showOptionalFields',
  'lark.hideOptionalFields',
  'dingtalk.clientId',
  'dingtalk.devConsoleLink',
  'dingtalk.clientIdDescSuffix',
  'dingtalk.clientSecret',
  'dingtalk.clientSecretDescSuffix',
  'dingtalk.credentialsRequired',
  'dingtalk.connectionSuccess',
  'dingtalk.connectionFailed',
  'dingtalk.pluginEnabled',
  'dingtalk.pluginDisabled',
  'dingtalk.enableFailed',
  'dingtalk.disableFailed',
  'dingtalk.testAndConnect',
  'dingtalk.credentialsSaved',
  'dingtalk.connectionStatus',
  'dingtalk.statusConnected',
  'dingtalk.statusError',
  'dingtalk.statusConnecting',
  'dingtalk.step1',
  'dingtalk.step2',
  'dingtalk.step3',
  'dingtalk.step4',
  'dingtalk.waitingConnection',
  'weixin.accountId',
  'weixin.scanPrompt',
  'weixin.loginButton',
  'weixin.connected',
  'weixin.disconnect',
  'weixin.scanned',
  'weixin.pluginEnabled',
  'weixin.pluginDisabled',
  'weixin.enableFailed',
  'weixin.disableFailed',
  'weixin.loginExpired',
  'weixin.loginError',
  'weixin.step1',
  'weixin.step2',
  'weixin.step3',
  'weixin.loginRequired',
  'wecom.wsTitle',
  'wecom.wsHint',
  'wecom.devDocLink',
  'wecom.botId',
  'wecom.botIdDesc',
  'wecom.secret',
  'wecom.secretDesc',
  'wecom.credentialsRequired',
  'wecom.credentialsSaved',
  'wecom.saveAndEnable',
  'wecom.pluginEnabled',
  'wecom.pluginDisabled',
  'wecom.enableFailed',
  'wecom.disableFailed',
  'wecom.configureFirst',
  'wecom.connectionStatus',
  'wecom.statusConnected',
  'wecom.statusError',
  'wecom.statusConnecting',
  'wecom.step1',
  'wecom.step2',
  'wecom.step3',
  'wecom.step4',
  'wecom.waitingConnection',
] as const;

const NOMI_REMOTE_KEYS = [
  'settings.remoteStatusNotConfigured',
  'settings.remoteStatusDisabled',
  'settings.remoteStatusEnabled',
  'settings.remoteStatusRunning',
  'settings.remoteUnbindRow',
  'settings.remoteDeleteBot',
  'settings.remoteDeleteConfirm',
  'settings.remoteUnboundBot',
  'settings.remoteBotIdentity',
  'settings.remoteUnbindSuccess',
  'settings.remoteBindFailed',
  'settings.remotePending',
  'settings.remoteConfigure',
  'settings.remoteConfigTitle',
  'settings.remoteEnableChannel',
  'settings.remoteEnableChannelHint',
] as const;

const PUBLIC_COMPANION_CHANNEL_KEYS = [
  'channels.title',
  'channels.desc',
  'channels.bindRow',
  'channels.createBot',
  'channels.bindConfirm',
  'channels.unbindConfirm',
  'channels.bindSuccess',
  'channels.otherBots',
  'channels.modelFollowsAgent',
  'channels.footerHint',
] as const;

const COMMON_KEYS = ['copySuccess', 'refresh', 'unknownUser', 'unit.minute_short'] as const;
const CONVERSATION_KEYS = ['workspace.refresh'] as const;

function getLocaleValue(locale: LocaleJson, key: string): unknown {
  if (Object.prototype.hasOwnProperty.call(locale, key)) return locale[key];

  let cursor: unknown = locale;
  for (const segment of key.split('.')) {
    if (!cursor || typeof cursor !== 'object' || !Object.prototype.hasOwnProperty.call(cursor, segment)) {
      return undefined;
    }
    cursor = (cursor as LocaleJson)[segment];
  }
  return cursor;
}

function flattenLeafKeys(value: unknown, prefix = '', out: string[] = []): string[] {
  if (Array.isArray(value)) {
    value.forEach((item, index) => flattenLeafKeys(item, `${prefix}.${index}`, out));
  } else if (value && typeof value === 'object') {
    for (const [key, item] of Object.entries(value)) {
      flattenLeafKeys(item, prefix ? `${prefix}.${key}` : key, out);
    }
  } else {
    out.push(prefix);
  }
  return out;
}

function assertStringKeys(localeName: string, locale: LocaleJson, keys: readonly string[]) {
  const failures: string[] = [];
  for (const key of keys) {
    const value = getLocaleValue(locale, key);
    if (value === undefined) {
      failures.push(`${localeName} missing ${key}`);
    } else if (typeof value !== 'string') {
      failures.push(`${localeName} ${key} should be a string`);
    } else if (!value.trim()) {
      failures.push(`${localeName} ${key} should not be blank`);
    }
  }
  expect(failures).toEqual([]);
}

function assertLocaleKeyParity(label: string, left: LocaleJson, right: LocaleJson) {
  const leftKeys = flattenLeafKeys(left).sort();
  const rightKeys = flattenLeafKeys(right).sort();
  const leftOnly = leftKeys.filter((key) => !rightKeys.includes(key));
  const rightOnly = rightKeys.filter((key) => !leftKeys.includes(key));
  expect({ label, leftOnly, rightOnly }).toEqual({ label, leftOnly: [], rightOnly: [] });
}

describe('channel configuration locale coverage', () => {
  test('public companion locales keep matching key coverage', () => {
    assertLocaleKeyParity('publicCompanion', enPublicCompanion, zhPublicCompanion);
  });

  test('settings channel forms have complete en-US copy', () => {
    assertStringKeys('en-US settings', enSettings, CHANNEL_SETTINGS_KEYS);
  });

  test('settings channel forms have complete zh-CN copy', () => {
    assertStringKeys('zh-CN settings', zhSettings, CHANNEL_SETTINGS_KEYS);
  });

  test('public companion channel surfaces have complete locale copy', () => {
    assertStringKeys('en-US publicCompanion', enPublicCompanion, PUBLIC_COMPANION_CHANNEL_KEYS);
    assertStringKeys('zh-CN publicCompanion', zhPublicCompanion, PUBLIC_COMPANION_CHANNEL_KEYS);
    assertStringKeys('en-US nomi', enNomi, NOMI_REMOTE_KEYS);
    assertStringKeys('zh-CN nomi', zhNomi, NOMI_REMOTE_KEYS);
    assertStringKeys('en-US common', enCommon, COMMON_KEYS);
    assertStringKeys('zh-CN common', zhCommon, COMMON_KEYS);
    assertStringKeys('en-US conversation', enConversation, CONVERSATION_KEYS);
    assertStringKeys('zh-CN conversation', zhConversation, CONVERSATION_KEYS);
  });
});
