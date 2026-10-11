/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const readSource = (name: string): string =>
  readFileSync(new URL(`./${name}`, import.meta.url), 'utf8');

describe('Relay pairing UI contract', () => {
  test('keeps the envelope opaque and delegates Mobile pairing to its task panel', () => {
    const source = readSource('RelayPairingPanel.tsx');

    expect(source.includes('pairingEnvelope')).toBe(true);
    expect(source.includes('nomifun-relay-pair:v2:')).toBe(true);
    expect(source.includes('QRCode')).toBe(false);
    expect(source.includes('localStorage')).toBe(false);
    expect(source.includes('console.log')).toBe(false);
    expect(source.includes('enrol_token')).toBe(false);
    expect(source.includes('relayPauseHint')).toBe(true);
    expect(source.includes('relayRemoveLegacyConfirm')).toBe(true);
  });

  test('uses the dedicated Relay bridge instead of the WebUI login QR endpoint', () => {
    const source = readSource('RelayPairingPanel.tsx');

    expect(source.includes('ipcBridge.relayPairing.bootstrap.invoke')).toBe(true);
    expect(source.includes('generateQRToken')).toBe(false);
    expect(source.includes('/qr-login?token=')).toBe(false);
  });

  test('polls restored state and exposes managed restart and disconnect actions', () => {
    const source = readSource('RelayPairingPanel.tsx');

    expect(source.includes('window.setInterval')).toBe(true);
    expect(source.includes("runAction('restart')")).toBe(true);
    expect(source.includes("runAction('disconnect')")).toBe(true);
    expect(source.includes("runAction('stop')")).toBe(true);
    expect(source.includes('Modal.confirm')).toBe(true);
  });
});
