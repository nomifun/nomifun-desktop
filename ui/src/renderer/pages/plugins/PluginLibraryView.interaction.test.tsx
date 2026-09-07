/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import {
  parsePluginArtifactId,
  parsePluginMountId,
} from '@/common/types/ids';
import type { PluginDetail } from '@/common/types/pluginPlatform';
import en from '@/renderer/services/i18n/locales/en-US/pluginWorkbench.json';
import PluginLibraryView from './PluginLibraryView';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {
        pluginWorkbench: en,
      },
    },
  },
  interpolation: { escapeValue: false },
});

const mountId = parsePluginMountId('0190f5fe-7c00-7a00-8000-000000000041');
const artifactId = parsePluginArtifactId('0190f5fe-7c00-7a00-8000-000000000042');

const detail = (installed: boolean): PluginDetail => ({
  summary: {
    mount_id: mountId,
    mount_revision: 3,
    display_name: 'Discoverable Plugin',
    lifecycle: installed ? 'enabled' : 'uninstalled_data_retained',
    current: installed
      ? {
          package_id: 'dev.nomifun.discoverable',
          package_version: '1.0.0',
          artifact_id: artifactId,
          artifact_digest: 'a'.repeat(64),
          manifest_digest: 'b'.repeat(64),
        }
      : undefined,
    contribution_count: 0,
    updated_at_ms: 1,
  },
  capabilities: [],
  config_schema: {
    schema_digest: 'c'.repeat(64),
    schema: {
      type: 'object',
      additionalProperties: false,
    },
  },
  config: {
    config_revision: 1,
    schema_digest: 'c'.repeat(64),
    values: {},
    valid: true,
    validation_errors: [],
  },
  credential_bindings_revision: 1,
  credential_slots: [],
  retained_data: !installed,
});

const renderLibrary = (plugin: PluginDetail, onConfigure: () => void) =>
  render(
    <I18nextProvider i18n={testI18n}>
      <PluginLibraryView
        plugins={[plugin.summary]}
        selectedMountId={mountId}
        detail={plugin}
        detailLoading={false}
        detailFailure={null}
        mutationFailure={null}
        busyAction={null}
        locale='en-US'
        onSelect={() => {}}
        onRetryDetail={() => {}}
        onConfigure={onConfigure}
        onEnable={() => {}}
        onDisable={() => {}}
        onRetryMount={() => {}}
        onRestore={() => {}}
        onUninstall={() => {}}
        onDeleteData={() => {}}
      />
    </I18nextProvider>
  );

afterEach(() => cleanup());

describe('Plugin Library configuration entry', () => {
  test('keeps config and Credential editing visible in the primary action bar', () => {
    let opened = 0;
    const page = renderLibrary(detail(true), () => {
      opened += 1;
    });

    const trigger = page.getByRole('button', {
      name: 'Config & credentials',
    });
    fireEvent.click(trigger);
    expect(opened).toBe(1);
  });

  test('keeps the entry discoverable but disabled without an installed Current target', () => {
    const page = renderLibrary(detail(false), () => {});
    const trigger = page.getByRole('button', {
      name: 'Config & credentials',
    });

    expect(trigger.hasAttribute('disabled')).toBe(true);
    expect(trigger.getAttribute('title')).toBe(
      'An installed Current version is required before this Plugin can be configured.'
    );
  });
});
