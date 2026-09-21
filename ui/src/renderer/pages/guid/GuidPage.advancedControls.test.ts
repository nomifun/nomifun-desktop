/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('GuidPage advanced controls', () => {
  test('keeps only the supported session-specific draft controls', () => {
    const source = readSource(new URL('./GuidPage.tsx', import.meta.url));
    const send = readSource(new URL('./hooks/useGuidSend.ts', import.meta.url));

    expect(source.includes('<AutoWorkControl')).toBe(true);
    expect(source.includes('<IdmmControl')).toBe(true);
    expect(source.includes('<AgentResourcePicker')).toBe(true);
    expect(source.includes('<KnowledgeControl')).toBe(true);
    expect(source.includes('knowledgeEnabled && (')).toBe(true);
    expect(source.indexOf('<KnowledgeControl')).toBeLessThan(
      source.indexOf('<AutoWorkControl')
    );
    expect(source.includes('openBrowserHandler')).toBe(false);
    expect(source.includes('isBrowserButtonDisabled')).toBe(false);
    expect(send.includes("launch('browser')")).toBe(false);
    expect(send.includes('initial-browser-open')).toBe(false);
    expect(send.includes("'message' | 'browser'")).toBe(false);
    expect(source.includes('<SessionCapabilityPicker')).toBe(false);
    expect(source.includes('useSessionCapabilityCatalog')).toBe(false);
    expect(send.includes('capability_selection')).toBe(false);
  });

  test('keeps the remaining draft API focused on session behavior', () => {
    const source = readSource(new URL('./hooks/useGuidSessionOptions.ts', import.meta.url));

    expect(source.includes('autoWork: AutoWorkDraftValue')).toBe(true);
    expect(source.includes('idmm: IIdmmConfig')).toBe(true);
    expect(source.includes('setIdmmDefault: (next: IIdmmConfig) => void')).toBe(true);
    expect(source.includes('idmmOverriddenRef.current')).toBe(true);
    expect(source.includes('knowledge: IKnowledgeBinding')).toBe(true);
  });

  test('shows collaboration only when the selected Agent grants its Module', () => {
    const source = readSource(new URL('./GuidPage.tsx', import.meta.url));

    expect(source.includes("presetCapabilityIds.has('agent.collaboration')")).toBe(true);
    expect(source.includes('collaboration: collaborationEnabled ? collaboration.config : undefined')).toBe(true);
    expect(source.includes('collaborationEnabled && <ComposerToolRail')).toBe(true);
    expect(source.includes('disabled={advancedConfig.autoWork.enabled}')).toBe(true);
  });

  test('exposes target resource controls from the capability contract while preserving explicit project context', () => {
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));
    const capabilityHook = readSource(
      new URL('./hooks/useGuidPresetCapabilities.ts', import.meta.url)
    );

    expect(page.includes('const workspaceEnabled =')).toBe(true);
    expect(page.includes('Boolean(guidInput.dir.trim()) ||')).toBe(true);
    expect(page.includes("presetResourceKinds.has('workspace')")).toBe(true);
    expect(page.includes("presetResourceKinds.has('knowledge_base')")).toBe(true);
    expect(page.includes("filter((kind) => kind !== 'knowledge_base')")).toBe(true);
    expect(
      page.includes('requiredKinds={resourcePickerKinds}')
    ).toBe(true);
    expect(page.includes('optionalKinds={optionalResourcePickerKinds}')).toBe(true);
    expect(page.includes('{workspaceEnabled && <GuidWorkspaceFootnote')).toBe(true);
    expect(page.includes('resourceSelections: resourceSelectionResolution.selections')).toBe(true);
    expect(page.includes('resourceSelectionResolution.missingKinds.length === 0')).toBe(true);
    expect(capabilityHook.includes('editor.revision?.document ?? editor.draft.document')).toBe(
      true
    );
    expect(capabilityHook.includes('requiredResourceKindsForDocument')).toBe(true);
    expect(capabilityHook.includes('document.runtime_policy?.idmm')).toBe(true);
    expect(page.includes('presetCapabilities.idmm')).toBe(true);
    expect(page.includes('const idmmControlEnabled = presetResourceResolutionReady;')).toBe(true);
    expect(page.includes('isCompanionAgent')).toBe(false);
  });

  test('shows a visible error when selected Preset capabilities cannot be resolved', () => {
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));

    expect(page.includes("import { Alert, ConfigProvider } from '@arco-design/web-react';")).toBe(
      true
    );
    expect(
      page.includes(
        "agentSelection.selection.kind === 'preset' && presetCapabilities.error"
      )
    ).toBe(true);
    expect(page.includes("<Alert\n                type='error'")).toBe(true);
    expect(page.includes("title={t('common.error')}")).toBe(true);
    expect(
      page.includes("content={t('agentSettings.errors.presetCapabilitiesLoadFailed')}")
    ).toBe(true);
  });

  test('blocks a tool-using Agent before launch when the selected model cannot call tools', () => {
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));
    const notice = readSource(new URL('./components/GuidModelCompatibilityNotice.tsx', import.meta.url));
    const modelSelection = readSource(new URL('./hooks/useGuidModelSelection.ts', import.meta.url));

    expect(page.includes('selectedAgentRequiresToolCalls = presetActionIds.size > 0')).toBe(true);
    expect(page.includes("? ['function_calling']")).toBe(true);
    expect(page.includes('const missingModelTraits = requiredModelTraits.filter(')).toBe(true);
    expect(page.includes('selectedAgentModelCompatible')).toBe(true);
    expect(page.includes('<GuidModelCompatibilityNotice')).toBe(true);
    expect(page.includes('modelCapabilityConfigurationRoute(currentModel.id, currentModel.use_model)')).toBe(true);
    expect(page.includes('onChooseCompatibleModel={() => setModelPickerOpen(true)}')).toBe(true);
    expect(notice.includes('missingTraits.map((trait)')).toBe(true);
    expect(notice.includes("t(`settings.modelTrait.${trait}`)")).toBe(true);
    expect(modelSelection.includes("capabilityOf(provider, current_model.use_model, 'chat')?.traits")).toBe(true);
  });
});
