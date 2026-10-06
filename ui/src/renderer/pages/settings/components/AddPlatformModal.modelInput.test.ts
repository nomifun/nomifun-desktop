/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const addSource = readFileSync(new URL('./AddPlatformModal.tsx', import.meta.url), 'utf8');
const editorSource = readFileSync(new URL('./ModelDefinitionEditor.tsx', import.meta.url), 'utf8');

describe('AddPlatformModal unified model input flow', () => {
  test('starts with model ID entry and an unfiltered provider catalog', () => {
    expect(editorSource.includes("t('settings.modelSupportedTasks'")).toBe(false);
    expect(editorSource.includes('<AutoComplete')).toBe(true);
    expect(editorSource.includes('catalogSuggestions')).toBe(true);
    expect(editorSource.includes('data-model-catalog-status')).toBe(true);
    expect(editorSource.includes("t('settings.modelCatalogUnavailable'")).toBe(false);
    expect(editorSource.includes('catalogSuggestionsForTask')).toBe(false);
    expect(editorSource.includes('applyCatalogSuggestion(')).toBe(true);
    expect(editorSource.includes('data={catalogSuggestions.map')).toBe(true);
    expect(editorSource.includes('data-unified-model-input')).toBe(true);
    expect(editorSource.includes('data-model-catalog-picker')).toBe(false);
    const unifiedInputSource = editorSource.slice(
      editorSource.indexOf('<AutoComplete'),
      editorSource.indexOf('data-unified-model-input')
    );
    expect(unifiedInputSource.includes('defaultActiveFirstOption={false}')).toBe(true);
    expect(unifiedInputSource.includes('onSelect=')).toBe(true);
    expect(unifiedInputSource.includes('onChange=')).toBe(true);
    expect(unifiedInputSource.includes('onBlur=')).toBe(false);
    expect(editorSource.includes('removeCapabilityTask(current.capabilities, task)')).toBe(true);
    expect(editorSource.includes('onChange((current) =>')).toBe(true);
    expect(editorSource.indexOf('data-unified-model-input')).toBeLessThan(
      editorSource.indexOf('data-model-call-route-picker')
    );
    expect(editorSource.includes('data-model-task-picker')).toBe(false);
    expect(editorSource.includes('data-model-traits-picker')).toBe(false);
    expect(editorSource.includes('removeModelTaskConfirm')).toBe(true);
    expect(editorSource.includes('modelTask.registered')).toBe(false);
    expect(editorSource.includes('data-remove-model-task={capability.task}')).toBe(true);
  });

  test('keeps generic purpose unconfirmed while preserving specialized and deep-link intent', () => {
    expect(editorSource.includes('primaryTask')).toBe(false);
    expect(editorSource.includes('data-primary-model-task-picker')).toBe(false);
    expect(editorSource.includes('data-primary-model-task-section')).toBe(false);
    expect(editorSource.includes('changePrimaryModelTask')).toBe(false);
    expect(editorSource.includes("t('settings.modelType'")).toBe(false);
    expect(addSource.includes('createModelDefinitionDraft(deepLinkData?.task ?? initialTask)')).toBe(true);
    expect(addSource.includes('initialTask?: ModelTask')).toBe(true);
    expect(addSource.includes("emptyCapabilityDraft('chat')")).toBe(false);
    expect(addSource.includes("deepLinkData?.task ?? 'chat'")).toBe(false);
    expect(addSource.includes('tasksSource: model.tasksSource')).toBe(true);
    expect(editorSource.includes('data-add-call-route')).toBe(true);
    expect(editorSource.includes('data-model-call-route-picker')).toBe(true);
    expect(editorSource.includes('capabilities: addCapabilityTask(current.capabilities, task)')).toBe(true);
  });

  test('gets operational defaults only from the backend preset manifest', () => {
    expect(addSource.includes('useModelProtocolManifests({')).toBe(true);
    expect(addSource.includes("bootstrapTask: 'chat'")).toBe(true);
    expect(addSource.includes('modelHint: definition.model')).toBe(false);
    expect(addSource.includes('providerManifest.platform_default_base_url')).toBe(true);
    expect(addSource.includes('providerManifest.default_auth_scheme')).toBe(true);
    expect(addSource.includes('providerManifest?.auth_schemes')).toBe(true);
    expect(addSource.includes('buildAuthSchemeOptions(')).toBe(true);
    expect(addSource.includes('filterOption={false}')).toBe(true);
    expect(addSource.includes('manifestState.loadingTasks.length > 0')).toBe(true);
    expect(
      editorSource.includes('const manifest = loading ? undefined : manifests[capability.task]')
    ).toBe(true);
  });

  test('auto-detects Custom and New API transport before save', () => {
    expect(addSource.includes('useProviderAutoConfiguration({')).toBe(true);
    expect(addSource.includes('applyProviderAutoConfiguration')).toBe(true);
    expect(addSource.includes('<ProviderAutoConfigurationNotice')).toBe(true);
    expect(addSource.includes('primary?.suggestedBaseUrl')).toBe(true);
    expect(addSource.includes("form.setFieldValue('auth_scheme', primary.authScheme)")).toBe(true);
    expect(addSource.includes('validationPending={autoConfiguration.isLoading}')).toBe(true);
  });

  test('offers one-click OpenAI and Claude interface presets', () => {
    expect(addSource.includes('<ProviderCompatibilityModePicker')).toBe(true);
    expect(addSource.includes('applyProviderCompatibilityMode')).toBe(true);
    expect(addSource.includes('providerCompatibilityAuthScheme')).toBe(true);
    expect(addSource.includes('normalizeProviderBaseUrlForCompatibilityMode')).toBe(true);
    expect(addSource.includes("compatibilityMode !== 'anthropic'")).toBe(true);
    expect(addSource.includes('protocolPreferences: compatibilityProtocolPreferences')).toBe(true);
  });

  test('keeps SDK-backed Bedrock providers free of transport URLs', () => {
    expect(addSource.includes("base_url: isBedrock ? '' :")).toBe(true);
    expect(addSource.includes('hidden={isBedrock}')).toBe(true);
  });

  test('persists exactly one atomic provider graph', () => {
    expect(addSource.includes('ipcBridge.mode.createProvider.invoke')).toBe(true);
    expect(addSource.includes('initial_model')).toBe(true);
    expect(addSource.includes('connections: pendingConnections')).toBe(true);
    expect(addSource.includes('capabilities,')).toBe(true);
  });

  test('a mixed-provider protocol can create and select an arbitrary named connection', () => {
    expect(editorSource.includes('data-create-named-connection={capability.task}')).toBe(true);
    expect(editorSource.includes('roleReadOnly')).toBe(true);
    expect(editorSource.includes('onCreateConnection(connection)')).toBe(true);
    expect(editorSource.includes('connectionRole: connection.role')).toBe(true);
    expect(editorSource.includes('providerAuthScheme ||')).toBe(true);
    expect(addSource.includes('setPendingConnections')).toBe(true);
    expect(addSource.includes('pendingConnections.map((connection) => connection.role)')).toBe(
      true
    );
  });
});
