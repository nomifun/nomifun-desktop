/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('execution transcript capability boundary', () => {
  test('marks every projected platform chat as read-only', () => {
    const source = readSource(new URL('./ReadOnlyConversationView.tsx', import.meta.url));
    // One readOnly / hideSendBox prop per projected platform arm. Only the nomi
    // arm remains, so each prop must appear exactly once — a second occurrence
    // would mean an unaudited second surface was reintroduced.
    expect(source.match(/readOnly/g)?.length ?? 0).toBe(1);
    expect(source.match(/hideSendBox/g)?.length ?? 0).toBe(1);
    expect(source.includes("creationTasksEnabled={conversation.agent_snapshot?.enabled_capabilities.includes('creation.media') === true}")).toBe(true);
  });

});
