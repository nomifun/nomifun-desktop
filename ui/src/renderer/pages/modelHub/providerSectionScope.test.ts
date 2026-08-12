/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const provider = readFileSync(
  new URL('../../components/settings/SettingsModal/contents/ModelModalContent.tsx', import.meta.url),
  'utf8'
);

describe('providers & keys section scope', () => {
  test('states where "find a model by purpose" moved to', () => {
    // Without this line the page still reads as the place to shop for a model,
    // and users keep hunting for the voice/vision settings inside a provider card.
    expect(provider.includes("t('settings.modelHub.provider.scopeNote')")).toBe(true);
  });

  test('keeps its actual job: the two-level provider/model list and the credential editors', () => {
    expect(provider.includes('SortableProviderCard')).toBe(true);
    expect(provider.includes('SortableModelRow')).toBe(true);
    // Credentials are edited through the provider dialog (which owns the API-key
    // editor) and the per-role connection profiles.
    expect(provider.includes('AddPlatformModal')).toBe(true);
    expect(provider.includes('ProviderConnectionsSection')).toBe(true);
    // The canonical capability editor and task-scoped rows stay here.
    expect(provider.includes('ModelAdvancedEditor')).toBe(true);
    expect(provider.includes('row.capabilities')).toBe(true);
  });
});
