/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { detectFamily } from './detectFamily';

describe('detectFamily', () => {
  test('detects direct and path-qualified invocations', () => {
    expect(detectFamily('claude')).toBe('claude');
    expect(detectFamily('/usr/local/bin/codex')).toBe('codex');
    expect(detectFamily('gemini --yolo')).toBe('gemini');
  });

  test('detects wrapped invocations via any token', () => {
    expect(detectFamily('stepcode claude')).toBe('claude');
    expect(detectFamily('npx codex')).toBe('codex');
  });

  test('returns null for unknown CLIs / shells', () => {
    expect(detectFamily('')).toBeNull();
    expect(detectFamily('/bin/bash -l')).toBeNull();
    expect(detectFamily('stepcode frobnicate')).toBeNull();
  });
});
