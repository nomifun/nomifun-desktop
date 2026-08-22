/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./ThreeDirectorRuntime.ts', import.meta.url), 'utf8');

describe('ThreeDirectorRuntime architecture', () => {
  test('owns a real renderer, controls, GLTF loader, resize observer, and animation loop', () => {
    for (const contract of [
      'new WebGLRenderer',
      'new OrbitControls',
      'new GLTFLoader',
      'new ResizeObserver',
      'requestAnimationFrame',
      'renderer.dispose()',
      'forceContextLoss()',
      'captureImage',
      'createDirectorRuntimeFramePlan',
    ]) {
      expect(source.includes(contract)).toBe(true);
    }
  });

  test('does not embed models, placeholder media, or data URLs', () => {
    expect(source.includes('.glb')).toBe(false);
    expect(/['"]data:/u.test(source)).toBe(false);
    expect(source.includes('base64')).toBe(false);
    expect(source.includes('placeholder')).toBe(false);
  });
});
