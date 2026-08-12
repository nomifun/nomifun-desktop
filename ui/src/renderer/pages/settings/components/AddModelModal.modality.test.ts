/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const read = (relative: string): string => readFileSync(new URL(relative, import.meta.url), 'utf8');

describe('single-source model management integration', () => {
  test('add-model uses the shared editor and one full capability save', () => {
    const source = read('./AddModelModal.tsx');
    expect(source.includes('<ModelDefinitionEditor')).toBe(true);
    expect(source.includes('useModelProtocolManifests')).toBe(true);
    expect(source.includes('ipcBridge.providerModel.save.invoke')).toBe(true);
    expect(source.includes('capabilityInputsFromDefinition')).toBe(true);
    expect(source.includes('ipcBridge.providerConnection.save.invoke')).toBe(true);
    expect(source.includes('provider_id: data.id')).toBe(true);
    expect(source.includes('catalogSuggestions={catalogSuggestions}')).toBe(true);
    expect(source.includes('const capabilities = capabilityInputsFromDefinition(definition)')).toBe(true);
    expect(source.includes('capabilities,')).toBe(true);
  });

  test('add-provider atomically sends provider, initial model, and named connections', () => {
    const source = read('./AddPlatformModal.tsx');
    expect(source.includes('<ModelDefinitionEditor')).toBe(true);
    expect(source.includes('ipcBridge.mode.createProvider.invoke')).toBe(true);
    expect(source.includes('initial_model')).toBe(true);
    expect(source.includes('connections: pendingConnections')).toBe(true);
    expect(source.includes('auth_scheme')).toBe(true);
    expect(source.includes('platform_default_base_url')).toBe(true);
    expect(source.includes('model: normalizeModelId(definition.model)')).toBe(true);
    expect(source.includes('catalogSuggestions={catalogSuggestions}')).toBe(true);
    expect(source.includes('const capabilities = capabilityInputsFromDefinition(definition)')).toBe(true);
    expect(source.includes('capabilities,')).toBe(true);
  });

  test('existing rows use nested capabilities and the same full-save editor', () => {
    const source = read('../../../components/settings/SettingsModal/contents/ModelModalContent.tsx');
    expect(source.includes('row.capabilities')).toBe(true);
    expect(source.includes('<ModelAdvancedEditor')).toBe(true);
    expect(source.includes('ipcBridge.providerModel.save.invoke')).toBe(true);
    expect(source.includes('capabilities: row.capabilities.map(capabilityInputFromResponse)')).toBe(true);
    expect(source.includes('updateModelCapabilities')).toBe(true);
  });
});
