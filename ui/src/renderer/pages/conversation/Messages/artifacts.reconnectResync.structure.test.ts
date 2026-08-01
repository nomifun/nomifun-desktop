/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./artifacts.tsx', import.meta.url), 'utf8');

describe('conversation artifacts reconnect recovery', () => {
  test('reloads the artifact list after websocket reconnect', () => {
    // WebSocket delivery has no replay: an artifactStream frame lost during a
    // gap is otherwise never recovered — re-run the listArtifacts snapshot load.
    expect(source.includes('const loadArtifacts = useCallback(')).toBe(true);
    expect(source.includes('ipcBridge.conversation.reconnected.on')).toBe(true);
  });
});
