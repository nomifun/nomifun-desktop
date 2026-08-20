/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const sources = [
  './DirectorWorkbenchShell.tsx',
  './DirectorSceneSidebar.tsx',
  './DirectorViewport.tsx',
  './DirectorInspector.tsx',
  './DirectorTimeline.tsx',
].map((path) => readFileSync(new URL(path, import.meta.url), 'utf8'));
const source = sources.join('\n');
const types = readFileSync(new URL('./types.ts', import.meta.url), 'utf8');
const css = readFileSync(new URL('./DirectorWorkbenchShell.module.css', import.meta.url), 'utf8');

describe('DirectorWorkbenchShell implementation boundary', () => {
  test('is a fully controlled presentation shell with caller-owned renderer slots', () => {
    expect(source.includes('useState')).toBe(false);
    expect(source.includes('useEffect')).toBe(false);
    expect(source.includes('useLayoutEffect')).toBe(false);
    expect(types.includes('viewportSlot: ReactNode')).toBe(true);
    expect(types.includes('gizmoSlot?: ReactNode')).toBe(true);
    expect(types.includes('viewportOverlaySlot?: ReactNode')).toBe(true);
    expect(source.includes('data-director-viewport-slot')).toBe(true);
    expect(source.includes('data-director-gizmo-slot')).toBe(true);
  });

  test('exposes scene, inspector, viewport, capture and timeline behavior through callbacks', () => {
    for (const callback of [
      'onViewModeChange',
      'onTransformModeChange',
      'onSceneObjectSelect',
      'onSceneObjectVisibilityChange',
      'onSceneObjectLockChange',
      'onInspectorChange',
      'onCaptureViewport',
      'onModelLibraryOpenChange',
      'onAspectRatioChange',
      'onRuleOfThirdsChange',
      'onTimelinePlayingChange',
      'onTimelineTimeChange',
      'onKeyframeSelect',
    ]) {
      expect(types.includes(callback)).toBe(true);
    }
    for (const inspectorKind of ['environment', 'camera', 'character', 'object']) {
      expect(types.includes(`kind: '${inspectorKind}'`)).toBe(true);
    }
  });

  test('does not embed transport, a second renderer, an iframe or fabricated 3D assets', () => {
    for (const forbidden of [
      'fetch(',
      'axios',
      '<iframe',
      '<canvas',
      'new WebGLRenderer',
      '@react-three/fiber',
      'three/examples',
      '.glb',
      '.gltf',
      'URL.createObjectURL',
    ]) {
      expect(source.includes(forbidden)).toBe(false);
    }
    expect(source.includes("from '@icon-park/react'")).toBe(true);
    expect(source.includes("from '@arco-design/web-react'")).toBe(true);
  });

  test('matches the source shell dimensions and its 920px compact behavior', () => {
    expect(css.includes('--director-left-width: 220px')).toBe(true);
    expect(css.includes('--director-right-width: 300px')).toBe(true);
    expect(css.includes('grid-template-rows: 70px minmax(0, 1fr)')).toBe(true);
    expect(css.includes('@media (max-width: 920px)')).toBe(true);
    expect(css.includes('grid-template-columns: min(220px, 28vw) minmax(0, 1fr)')).toBe(true);
    expect(css.includes('.inspector {\n    display: none;')).toBe(true);
    expect(css.includes('linear-gradient')).toBe(false);
    expect(css.includes('radial-gradient')).toBe(false);
  });
});
