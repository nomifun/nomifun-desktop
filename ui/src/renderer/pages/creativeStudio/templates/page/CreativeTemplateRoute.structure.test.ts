/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const route = readFileSync(new URL('./CreativeTemplateRoute.tsx', import.meta.url), 'utf8');
const page = readFileSync(
  new URL('./CreativeTemplateWorkspacePage.tsx', import.meta.url),
  'utf8'
);
const agentModal = readFileSync(
  new URL('./TemplateAgentDraftModal.tsx', import.meta.url),
  'utf8'
);
const editorModal = readFileSync(
  new URL('./TemplateEditorModal.tsx', import.meta.url),
  'utf8'
);

describe('Creative Template route composition', () => {
  test('uses the canonical repository and does not revive source local persistence', () => {
    expect(page.includes('creativeTemplateRepository')).toBe(true);
    expect(page.includes('localStorage')).toBe(false);
    expect(page.includes('localforage')).toBe(false);
    expect(page.includes('/api/v1/templates')).toBe(false);
    expect(route.includes("navigate('/models')")).toBe(true);
    expect(route.includes('useCreativeTemplateRuntime')).toBe(true);
    expect(route.includes('creativeAssetClient')).toBe(true);
    expect(route.includes('useCreativeAssetPickerDialog')).toBe(true);
    expect(route.includes('initialSelectedIds: selectedAssetIds')).toBe(true);
    expect(route.includes("acceptedKinds: ['image']")).toBe(true);
  });

  test('wires the minimal one-shot Template draft through the model catalog and backend port', () => {
    expect(route.includes('useNomiCreativeModelCatalog')).toBe(true);
    expect(route.includes('templateDraftPort')).toBe(true);
    expect(route.includes('agentDraftPort={templateDraftPort}')).toBe(true);
    expect(route.includes('agentModelCatalog={modelCatalog}')).toBe(true);
    expect(page.includes('<TemplateAgentDraftModal')).toBe(true);
    expect(page.includes('setEditing(withPrivateTemplateVisibility(template))')).toBe(true);
    expect(page.includes('setEditingIsNew(true)')).toBe(true);
    expect(agentModal.includes('repository.create')).toBe(false);
    expect(agentModal.includes('repository.save')).toBe(false);
    expect(agentModal.includes('conversation')).toBe(false);
  });

  test('keeps the launch UI private-only at every editor persistence boundary', () => {
    expect(editorModal.includes('可见范围')).toBe(false);
    expect(editorModal.includes('visibilitySwitch')).toBe(false);
    expect(editorModal.includes("visibility: 'public'")).toBe(false);
    expect(page.includes('template.metadata.visibility')).toBe(false);
    expect(page.match(/withPrivateTemplateVisibility/g)?.length ?? 0).toBeGreaterThanOrEqual(7);
    expect(page.includes('repository.create({ ...privateEditing, revision: 1 })')).toBe(true);
    expect(page.includes('repository.save(privateEditing.id, privateEditing.revision')).toBe(
      true
    );
  });

  test('derives template image dimensions from one fixed size selection', () => {
    expect(page.includes('modelCatalog={agentModelCatalog}')).toBe(true);
    expect(editorModal.includes('imageWorkbenchSizePolicyForModel')).toBe(true);
    expect(editorModal.includes('imageWorkbenchFixedSizeOptions')).toBe(true);
    expect(editorModal.includes("creativeStudio.templates.editor.width'")).toBe(false);
    expect(editorModal.includes("creativeStudio.templates.editor.height'")).toBe(false);
    expect(editorModal.includes('sizeOptionRow')).toBe(true);
  });
});
