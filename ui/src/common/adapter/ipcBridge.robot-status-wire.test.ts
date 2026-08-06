/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { InvalidEntityIdError } from '@/common/types/ids';
import { robot, type IApiRobot, type IApiRobotStatus } from './ipcBridge';

const source = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');
const COMPANION_ID = '0190f5fe-7c00-7a00-8000-0000000000c1';
const ROBOT_ID = 'aa:bb:cc:dd:ee:ff';
const realFetch = globalThis.fetch;

function respondWith(data: unknown): void {
  globalThis.fetch = (() =>
    Promise.resolve(
      new Response(JSON.stringify({ success: true, data }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    )) as unknown as typeof fetch;
}

const rawRobot = (companionId: unknown) => ({
  robot_id: ROBOT_ID,
  name: '书桌机器人',
  companion_id: companionId,
  board: 'esp32-s3n16r8-emoji',
  firmware_version: '1.9.0',
  last_seen: '2026-08-06T10:00:00Z',
  created_at: '2026-08-05T09:00:00Z',
});

describe('robot wire contract', () => {
  test('the six routes and the push event name are the ones the backend serves', () => {
    expect(source.includes("'/api/robots'")).toBe(true);
    expect(source.includes("'/api/robots/claim'")).toBe(true);
    expect(source.includes("'/api/robots/statuses'")).toBe(true);
    expect(source.includes("'/api/robots/endpoints'")).toBe(true);
    expect(source.includes('`/api/robots/${p.robot_id}`')).toBe(true);
    expect(source.includes("wsMappedEmitter<IApiRobotStatus>('robot.status'")).toBe(true);
  });

  test('robot_id is NOT branded — it is the device MAC, not a UUIDv7', () => {
    // Every other entity id in this bridge is a canonical UUIDv7 and gets a
    // parser. A robot is keyed by its Device-Id (MAC address) because that is
    // what the firmware reports, so branding it would reject every real device.
    expect(source.includes('parseRobotId')).toBe(false);
    expect(source.includes('robot_id: string;')).toBe(true);
  });

  test('every phase the backend can publish is a declared literal', () => {
    for (const phase of ['offline', 'idle', 'listening', 'speaking']) {
      expect(source.includes(`'${phase}'`)).toBe(true);
    }
    expect(source.includes('export type IApiRobotPhase')).toBe(true);
  });

  test('the snapshot and the push path share one mapper, so a status cannot differ by arrival route', () => {
    expect(source.includes('const fromApiRobotStatus')).toBe(true);
    expect(source.split('fromApiRobotStatus').length - 1).toBeGreaterThanOrEqual(3);
    expect(source.includes('changed_at: number;')).toBe(true);
  });

  test('the list route unwraps the {robots} envelope', async () => {
    try {
      respondWith({ robots: [rawRobot(COMPANION_ID)] });
      const rows: IApiRobot[] = await robot.list.invoke();
      expect(rows).toHaveLength(1);
      expect(rows[0]?.robot_id).toBe(ROBOT_ID);
      expect(rows[0]?.companion_id).toBe(COMPANION_ID);
      expect(rows[0]?.firmware_version).toBe('1.9.0');
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('an unbound robot arrives as an explicit null owner, and a legacy id is rejected', async () => {
    try {
      respondWith({ robots: [rawRobot(null)] });
      const unbound: IApiRobot[] = await robot.list.invoke();
      expect(unbound[0]?.companion_id).toBe(null);

      respondWith({ robots: [rawRobot(`companion_${COMPANION_ID}`)] });
      let error: unknown;
      try {
        await robot.list.invoke();
      } catch (caught) {
        error = caught;
      }
      expect(error instanceof InvalidEntityIdError).toBe(true);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('the statuses snapshot unwraps {statuses} and brands the owner', async () => {
    try {
      respondWith({
        statuses: [
          { robot_id: ROBOT_ID, companion_id: COMPANION_ID, phase: 'listening', changed_at: 7 },
        ],
      });
      const rows: IApiRobotStatus[] = await robot.statuses.invoke();
      expect(rows[0]?.phase).toBe('listening');
      expect(rows[0]?.changed_at).toBe(7);
      expect(rows[0]?.companion_id).toBe(COMPANION_ID);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('the endpoints route reports both the OTA candidates and the LAN switch', async () => {
    try {
      respondWith({ ota_urls: ['http://192.168.1.5:25808/robot/ota'], lan_enabled: false });
      const endpoints = await robot.endpoints.invoke();
      expect(endpoints.ota_urls).toEqual(['http://192.168.1.5:25808/robot/ota']);
      expect(endpoints.lan_enabled).toBe(false);
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
