/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const image = readFileSync(new URL('./ImageWorkbenchProductRoute.tsx', import.meta.url), 'utf8');
const video = readFileSync(new URL('./VideoWorkbenchProductRoute.tsx', import.meta.url), 'utf8');
const ownership = readFileSync(new URL('./ownership.ts', import.meta.url), 'utf8');
const shared = readFileSync(new URL('./shared.tsx', import.meta.url), 'utf8');
const wiring = readFileSync(new URL('./WIRING.md', import.meta.url), 'utf8');

describe('standalone workbench product wiring', () => {
  test('exports prop-free product routes composed from the source-parity views', () => {
    expect(image.includes('const ImageWorkbenchProductRoute: React.FC = () =>')).toBe(true);
    expect(image.includes('<ImageWorkbench {...props} />')).toBe(true);
    expect(video.includes('const VideoWorkbenchProductRoute: React.FC = () =>')).toBe(true);
    expect(video.includes('<VideoWorkbench {...props} />')).toBe(true);
  });

  test('fails closed without scope and wires durable lifecycle callbacks', () => {
    expect(ownership.includes("if (values.length === 0) return { state: 'missing'")).toBe(true);
    expect(image.includes('initialResumeRequests: persistence.initialResumeRequests')).toBe(true);
    expect(image.includes('onPendingTask: persistence.onPendingTask')).toBe(true);
    expect(video.includes('onSettledTask: persistence.onSettledTask')).toBe(true);
    expect(video.includes('onRecoveryFailure: persistence.onRecoveryFailure')).toBe(true);
    expect(shared.includes('navigate(CREATIVE_STUDIO_PROJECTS_PATH)')).toBe(true);
    expect(shared.includes("navigate('/workshop/projects')")).toBe(false);
    expect(wiring.includes('never borrows or creates a recent project implicitly')).toBe(true);
  });

  test('documents the durable owner foundation and coordinated route blocker', () => {
    expect(video.includes('STANDALONE_VIDEO_MAX_CONCURRENT_TASKS')).toBe(true);
    expect(
      wiring.includes('standalone_workbench { projectId, workbenchKind }')
    ).toBe(true);
    expect(wiring.includes('{ assetId, kind, role }')).toBe(true);
    expect(wiring.includes('inputs: null')).toBe(true);
    expect(wiring.includes('owner-scoped list/recovery')).toBe(true);
  });
});
