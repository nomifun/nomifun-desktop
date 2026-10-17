/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');

describe('model configuration page integration', () => {
  test('every model management form uses the content page', () => {
    for (const file of ['AddPlatformModal', 'AddModelModal', 'EditModeModal', 'EditModelGatewayModal', 'ModelAdvancedEditor', 'ProviderConnectionsSection']) {
      const source = read(`./${file}.tsx`);
      expect(source.includes('<ModelConfigurationPanel')).toBe(true);
      expect(source.includes('<NomiModal')).toBe(false);
      expect(source.includes('<Drawer')).toBe(false);
    }
  });

  test('accepts a one-shot deep-link request that opens the exact model editor', () => {
    const source = read('./ModelAdvancedEditor.tsx');
    expect(source.includes('openRequest?: string;')).toBe(true);
    expect(source.includes('handledOpenRequestRef.current === openRequest')).toBe(true);
    expect(source.includes('onOpenRequestHandled?.(openRequest)')).toBe(true);
  });
});
