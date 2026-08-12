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
  test('puts model identity before an explicit one-task-at-a-time capability flow', () => {
    expect(editorSource.includes("{t('settings.modelId'")).toBe(true);
    expect(editorSource.includes('<Input')).toBe(true);
    expect(editorSource.includes('catalogSuggestions')).toBe(true);
    expect(editorSource.includes('modelCatalogUnavailable')).toBe(true);
    expect(editorSource.includes('applyCatalogMetadata')).toBe(true);
    expect(editorSource.includes('applyCatalogSuggestion(suggestion)')).toBe(true);
    expect(editorSource.includes('addCapabilityTask(value.capabilities, task)')).toBe(true);
    expect(editorSource.includes('removeCapabilityTask(value.capabilities, task)')).toBe(true);
    expect(editorSource.indexOf("settings.modelId'")).toBeLessThan(
      editorSource.indexOf('data-model-task-section')
    );
    const taskPickerSource = editorSource.slice(
      editorSource.indexOf('data-model-task-section'),
      editorSource.indexOf('{value.capabilities.map')
    );
    expect(taskPickerSource.includes("mode='multiple'")).toBe(false);
    expect(editorSource.includes('modelTask.registered')).toBe(false);
    expect(editorSource.includes('data-remove-model-task={capability.task}')).toBe(true);
  });

  test('gets operational defaults only from the backend preset manifest', () => {
    expect(addSource.includes("useModelProtocolManifests(preset, tasks, 'chat')")).toBe(true);
    expect(addSource.includes('providerManifest.platform_default_base_url')).toBe(true);
    expect(addSource.includes('providerManifest.default_auth_scheme')).toBe(true);
    expect(addSource.includes('providerManifest?.auth_schemes')).toBe(true);
    expect(addSource.includes('manifestState.loadingTasks.length > 0')).toBe(true);
    expect(editorSource.includes('const manifest = loading ? undefined : manifests[capability.task]')).toBe(
      true
    );
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
    expect(editorSource.includes('authScheme={providerAuthScheme')).toBe(true);
    expect(addSource.includes('setPendingConnections')).toBe(true);
    expect(addSource.includes('pendingConnections.map((connection) => connection.role)')).toBe(true);
  });
});
