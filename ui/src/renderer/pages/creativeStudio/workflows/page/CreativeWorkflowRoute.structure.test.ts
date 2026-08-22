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
const editorModal = readFileSync(
  new URL('./WorkflowEditorModal.tsx', import.meta.url),
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
    expect(page.includes('setEditing(withPrivateWorkflowVisibility(workflow))')).toBe(true);
    expect(page.includes('setEditingIsNew(true)')).toBe(true);
    expect(agentModal.includes('repository.create')).toBe(false);
    expect(agentModal.includes('repository.save')).toBe(false);
    expect(agentModal.includes('conversation')).toBe(false);
  });

  test('keeps the launch UI private-only at every editor persistence boundary', () => {
    expect(editorModal.includes('可见范围')).toBe(false);
    expect(editorModal.includes('visibilitySwitch')).toBe(false);
    expect(editorModal.includes("visibility: 'public'")).toBe(false);
    expect(page.includes('workflow.metadata.visibility')).toBe(false);
    expect(page.match(/withPrivateWorkflowVisibility/g)?.length ?? 0).toBeGreaterThanOrEqual(7);
    expect(page.includes('repository.create({ ...privateEditing, revision: 1 })')).toBe(true);
    expect(page.includes('repository.save(privateEditing.id, privateEditing.revision')).toBe(
      true
    );
  });
});
