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

    expect(source.includes('<AutoWorkControl')).toBe(true);
    expect(source.includes('<IdmmControl')).toBe(true);
    expect(source.includes('<KnowledgeControl')).toBe(true);
    expect(source.includes('knowledgeEnabled && (')).toBe(true);
  });

  test('keeps the remaining draft API focused on session behavior', () => {
    const source = readSource(new URL('./hooks/useGuidAdvancedConfig.ts', import.meta.url));

    expect(source.includes('autoWork: AutoWorkDraftValue')).toBe(true);
    expect(source.includes('idmm: IIdmmConfig')).toBe(true);
    expect(source.includes('knowledge: IKnowledgeBinding')).toBe(true);
  });

  test('exposes target resource controls only from the selected preset capability contract', () => {
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));
    const capabilityHook = readSource(
      new URL('./hooks/useGuidPresetCapabilities.ts', import.meta.url)
    );

    expect(page.includes('const workspaceEnabled =')).toBe(true);
    expect(page.includes("presetResourceKinds.has('workspace')")).toBe(true);
    expect(page.includes("presetResourceKinds.has('knowledge_base')")).toBe(true);
    expect(page.includes('showWorkspace={workspaceEnabled}')).toBe(true);
    expect(capabilityHook.includes('editor.revision?.document ?? editor.draft.document')).toBe(
      true
    );
    expect(capabilityHook.includes('requiredResourceKindsForDocument')).toBe(true);
  });

  test('shows a visible error when selected Preset capabilities cannot be resolved', () => {
    const page = readSource(new URL('./GuidPage.tsx', import.meta.url));

    expect(page.includes("import { Alert, ConfigProvider } from '@arco-design/web-react';")).toBe(
      true
    );
    expect(page.includes('!isDefaultAgent && presetCapabilities.error')).toBe(true);
    expect(page.includes("<Alert\n                type='error'")).toBe(true);
    expect(page.includes("title={t('common.error')}")).toBe(true);
    expect(
      page.includes("content={t('agentSettings.errors.presetCapabilitiesLoadFailed')}")
    ).toBe(true);
  });
});
