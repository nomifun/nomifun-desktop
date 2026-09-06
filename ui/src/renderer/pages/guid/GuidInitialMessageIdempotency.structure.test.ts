/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL): string => readFileSync(url, 'utf8');

describe('Guid initial-message idempotency', () => {
  test('persists one UUIDv7 key for the single AgentPreset launch path', () => {
    const source = readSource(new URL('./hooks/useGuidSend.ts', import.meta.url));
    const initialMessagePayload =
      source.match(/JSON\.stringify\(\{([\s\S]*?)\}\)\s*\)/)?.[1] ?? '';

    expect(source.includes("import { uuidv7 } from '@/common/utils';")).toBe(true);
    expect(source.match(/idempotency_key: uuidv7\(\),/g)).toHaveLength(1);
    expect(initialMessagePayload).not.toBe('');
    expect(initialMessagePayload.match(/conversation_id: conversationId,/g)).toHaveLength(1);
    expect(initialMessagePayload.match(/initial_admission_epoch: 0,/g)).toHaveLength(1);
    expect(initialMessagePayload.match(/idempotency_key: uuidv7\(\),/g)).toHaveLength(1);
    expect(source.match(/'initial-message-nomi'/g)).toHaveLength(1);

    const writesBeforeNavigation =
      source.lastIndexOf('sessionStorage.setItem') < source.lastIndexOf('await navigate(');
    expect(writesBeforeNavigation).toBe(true);
  });

  test('Nomi QuickStart persists the auto-send key before navigation', () => {
    const source = readSource(
      new URL('../../hooks/agent/useNomiQuickStart.ts', import.meta.url)
    );
    const key = source.indexOf('idempotency_key: uuidv7()');
    const owner = source.indexOf('conversation_id: conversation.id');
    const epoch = source.indexOf('initial_admission_epoch: 0', owner);
    const storageWrite = source.indexOf('sessionStorage.setItem(');
    const navigation = source.indexOf('await navigate(');

    expect(
      source.includes("import { uuidv7 } from '@/common/utils/uuidv7';")
    ).toBe(true);
    expect(storageWrite >= 0).toBe(true);
    expect(owner > storageWrite).toBe(true);
    expect(epoch > owner).toBe(true);
    expect(key > storageWrite).toBe(true);
    expect(navigation > key).toBe(true);
  });
});
