/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import zhNomi from '@/renderer/services/i18n/locales/zh-CN/nomi.json';
import enNomi from '@/renderer/services/i18n/locales/en-US/nomi.json';

const section = readFileSync(new URL('./RobotConnectSection.tsx', import.meta.url), 'utf8');
const modal = readFileSync(new URL('./AddRobotModal.tsx', import.meta.url), 'utf8');
const tab = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const imSection = readFileSync(new URL('./RemoteConnectSection.tsx', import.meta.url), 'utf8');

describe('robot connect section', () => {
  test('lists only this companion’s robots and pills them from the live map', () => {
    expect(section.includes('useRobotStatuses()')).toBe(true);
    expect(section.includes('ROBOT_STATUS_COLOR')).toBe(true);
    expect(section.includes('row.companion_id === companionId')).toBe(true);
  });

  test('offers rename, unbind and delete, and delete is a danger confirm', () => {
    expect(section.includes('ipcBridge.robot.update.invoke')).toBe(true);
    expect(section.includes('companion_id: null')).toBe(true);
    expect(section.includes('ipcBridge.robot.remove.invoke')).toBe(true);
    expect(section.includes("okButtonProps: { status: 'danger' }")).toBe(true);
  });

  test('a failed list read renders an explanation, never a crash or a silent empty', () => {
    // Plan A's backend may not be deployed yet, and a 404 must read as "cannot
    // reach the robot service", not as "you own no robots".
    expect(section.includes("t('nomi.robot.loadFailed')")).toBe(true);
  });

  test('the add dialog shows every OTA candidate with a copy button and the 6-digit code field', () => {
    expect(modal.includes('ipcBridge.robot.endpoints.invoke()')).toBe(true);
    expect(modal.includes('<CopyIconButton')).toBe(true);
    expect(modal.includes('maxLength={6}')).toBe(true);
    expect(modal.includes('ipcBridge.robot.claim.invoke')).toBe(true);
  });

  test('a wrong code and an already-claimed device get their own message', () => {
    expect(modal.includes('claimNotFound')).toBe(true);
    expect(modal.includes('claimTaken')).toBe(true);
    expect(modal.includes('status === 404')).toBe(true);
    expect(modal.includes('status === 409')).toBe(true);
  });

  test('the LAN dependency is stated and can be switched on from the dialog', () => {
    expect(modal.includes('lan_enabled')).toBe(true);
    expect(modal.includes('webui.start.invoke')).toBe(true);
    expect(modal.includes('webui.lifecycleSupported')).toBe(true);
  });

  test('the tab renders the section and aggregates attention from BOTH sources', () => {
    expect(tab.includes('<RobotConnectSection')).toBe(true);
    expect(tab.includes('pendingPairings > 0 || robotAttention')).toBe(true);
  });

  test('the IM section says "IM robot" so the two never collide on screen', () => {
    // 「远程连接」节里的「机器人」一直指 IM bot; the new section is about physical
    // hardware, so the older copy has to name its own kind.
    expect(imSection.includes("t('nomi.settings.remoteCreateBot')")).toBe(true);
    const zh = (zhNomi as unknown as { settings: Record<string, string> }).settings;
    expect(zh.remoteCreateBot).toBe('连接 IM 机器人');
    expect(zh.remoteBotIdentity.startsWith('IM 机器人')).toBe(true);
  });

  test('robot copy is complete in both locales', () => {
    const keys = [
      'title',
      'hint',
      'add',
      'addTitle',
      'otaStep',
      'otaNone',
      'codeStep',
      'codePlaceholder',
      'claim',
      'claimOk',
      'claimNotFound',
      'claimTaken',
      'claimFailed',
      'lanOff',
      'lanEnable',
      'lanEnabled',
      'lanEnableFailed',
      'lanUnavailable',
      'empty',
      'board',
      'firmware',
      'lastSeen',
      'lastSeenNever',
      'rename',
      'renameTitle',
      'renamePlaceholder',
      'renameFailed',
      'unbind',
      'unbindConfirm',
      'remove',
      'removeConfirm',
      'removeFailed',
      'loadFailed',
    ];
    for (const locale of [zhNomi, enNomi]) {
      const robot = (locale as unknown as { robot: Record<string, unknown> }).robot;
      for (const key of keys) {
        expect(typeof robot[key]).toBe('string');
      }
      const status = robot.status as Record<string, string>;
      for (const phase of ['offline', 'idle', 'listening', 'speaking']) {
        expect(typeof status[phase]).toBe('string');
      }
    }
  });
});
