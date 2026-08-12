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
  test('puts model type before one filtered catalog and free-text input', () => {
    expect(editorSource.includes("t('settings.modelType'")).toBe(true);
    expect(editorSource.includes('<AutoComplete')).toBe(true);
    expect(editorSource.includes('catalogSuggestions')).toBe(true);
    expect(editorSource.includes('modelCatalogUnavailable')).toBe(true);
    expect(editorSource.includes('catalogSuggestionsForTask')).toBe(true);
    expect(editorSource.includes('applyCatalogSuggestionForTask')).toBe(true);
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
    expect(editorSource.includes('addCapabilityTask(value.capabilities, task)')).toBe(true);
    expect(editorSource.includes('removeCapabilityTask(value.capabilities, task)')).toBe(true);
    expect(editorSource.indexOf('data-primary-model-task-picker')).toBeLessThan(
      editorSource.indexOf('data-unified-model-input')
    );
    const taskPickerSource = editorSource.slice(
      editorSource.indexOf('data-model-task-section'),
      editorSource.indexOf('{value.capabilities.map')
    );
    expect(taskPickerSource.includes("mode='multiple'")).toBe(false);
    expect(editorSource.includes('modelTask.registered')).toBe(false);
    expect(editorSource.includes('data-remove-model-task={capability.task}')).toBe(true);
    expect(editorSource.includes('capability.task !== primaryTask')).toBe(true);
  });

  test('gets operational defaults only from the backend preset manifest', () => {
    expect(addSource.includes("useModelProtocolManifests(preset, tasks, 'chat')")).toBe(true);
    expect(addSource.includes('providerManifest.platform_default_base_url')).toBe(true);
    expect(addSource.includes('providerManifest.default_auth_scheme')).toBe(true);
    expect(addSource.includes('providerManifest?.auth_schemes')).toBe(true);
    expect(addSource.includes('manifestState.loadingTasks.length > 0')).toBe(true);
    expect(
      editorSource.includes('const manifest = loading ? undefined : manifests[capability.task]')
    ).toBe(true);
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
