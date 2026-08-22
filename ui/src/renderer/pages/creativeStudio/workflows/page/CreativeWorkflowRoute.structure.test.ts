/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const route = readFileSync(new URL('./CreativeWorkflowRoute.tsx', import.meta.url), 'utf8');
const page = readFileSync(
  new URL('./CreativeWorkflowWorkspacePage.tsx', import.meta.url),
  'utf8'
);
const agentModal = readFileSync(
  new URL('./WorkflowAgentDraftModal.tsx', import.meta.url),
  'utf8'
);

describe('Creative Workflow route composition', () => {
  test('uses the canonical repository and does not revive source local persistence', () => {
    expect(page.includes('creativeWorkflowRepository')).toBe(true);
    expect(page.includes('localStorage')).toBe(false);
    expect(page.includes('localforage')).toBe(false);
    expect(page.includes('/api/v1/workflows')).toBe(false);
    expect(route.includes("navigate('/models')")).toBe(true);
    expect(route.includes('useCreativeWorkflowRuntime')).toBe(true);
    expect(route.includes('creativeAssetClient')).toBe(true);
    expect(route.includes('useCreativeAssetPickerDialog')).toBe(true);
    expect(route.includes('initialSelectedIds: selectedAssetIds')).toBe(true);
    expect(route.includes("acceptedKinds: ['image']")).toBe(true);
  });

  test('wires the minimal one-shot Workflow draft through the model catalog and backend port', () => {
    expect(route.includes('useNomiCreativeModelCatalog')).toBe(true);
    expect(route.includes('workflowDraftPort')).toBe(true);
    expect(route.includes('agentDraftPort={workflowDraftPort}')).toBe(true);
    expect(route.includes('agentModelCatalog={modelCatalog}')).toBe(true);
    expect(page.includes('<WorkflowAgentDraftModal')).toBe(true);
    expect(page.includes('setEditing(workflow)')).toBe(true);
    expect(page.includes('setEditingIsNew(true)')).toBe(true);
    expect(agentModal.includes('repository.create')).toBe(false);
    expect(agentModal.includes('repository.save')).toBe(false);
    expect(agentModal.includes('conversation')).toBe(false);
  });
});
