/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const bridge = readFileSync(new URL('../../../common/adapter/ipcBridge.ts', import.meta.url), 'utf8');
const useNomi = readFileSync(new URL('./useNomi.ts', import.meta.url), 'utf8');

describe('companion model-slot wire mirror', () => {
  test('the profile declares every slot the Rust struct serializes', () => {
    // The backend serializes these unconditionally (no skip_serializing_if), so
    // the type must not mark them optional — an optional field here would let a
    // consumer read `undefined` where the wire always sends `null`.
    expect(bridge.includes('fallback_model: ICompanionModelRef | null;')).toBe(true);
    expect(bridge.includes('vision_model: ICompanionModelRef | null;')).toBe(true);
    expect(bridge.includes('voice: ICompanionVoiceConfig;')).toBe(true);
    expect(bridge.includes('export interface ICompanionVoiceConfig')).toBe(true);
    expect(bridge.includes('export interface ICompanionTtsSelection')).toBe(true);
    expect(bridge.includes('export interface ICompanionVadConfig')).toBe(true);
    expect(bridge.includes('min_silence_ms: number;')).toBe(true);
  });

  test('the patch type can address one voice sub-field at a time', () => {
    const start = bridge.indexOf('export type ICompanionProfilePatch');
    const patch = bridge.slice(start, bridge.indexOf('};', start));
    expect(patch.includes('fallback_model?: ICompanionModelRef | null;')).toBe(true);
    expect(patch.includes('vision_model?: ICompanionModelRef | null;')).toBe(true);
    expect(patch.includes('vad?: Partial<ICompanionVadConfig>;')).toBe(true);
  });

  test('the optimistic merge reaches two levels deep into voice', () => {
    // `voice.vad` is nested one level below `voice`; a single spread would
    // replace the whole vad block and blank the untouched parameter, so the
    // slider would visibly snap the other value back to its default.
    const start = useNomi.indexOf('const mergeProfile');
    const merge = useNomi.slice(start, useNomi.indexOf('});', start));
    expect(merge.includes('patch.fallback_model !== undefined')).toBe(true);
    expect(merge.includes('patch.vision_model !== undefined')).toBe(true);
    expect(merge.includes('vad: { ...prev.voice.vad, ...patch.voice.vad }')).toBe(true);
  });
});
