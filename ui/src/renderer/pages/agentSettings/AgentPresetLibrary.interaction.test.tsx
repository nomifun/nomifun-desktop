/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';

import {
  cleanup,
  render,
  within,
} from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import {
  asAgentPresetId,
  type AgentPresetLibraryResponse,
  type AgentPresetSummary,
} from '@/common/types/agentPlatform';
import AgentPresetLibrary from './AgentPresetLibrary';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {
        common: {
          cancel: 'Cancel',
        },
        agentSettings: {
          title: 'Agent Workbench',
          actions: {
            create: 'Create',
            delete: 'Delete',
          },
          defaults: {
            untitledName: 'Untitled Agent',
          },
          library: {
            ariaLabel: 'Agent library',
            official: 'Official templates',
            mine: 'My Agents',
            empty: 'No custom Agents yet.',
            capabilityCount: '{{count}} capabilities',
            deleteConfirmTitle: 'Delete “{{name}}”?',
            deleteConfirmBody: 'Existing conversation history is preserved.',
            deleteAria: 'Delete Agent “{{name}}”',
          },
          template: {
            chat: {
              minimal: {
                name: 'Minimal Chat',
              },
            },
          },
        },
      },
    },
  },
  interpolation: { escapeValue: false },
});

const preset: AgentPresetSummary = {
  preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001'),
  source: 'user',
  display_name: 'Research Agent',
  description: 'Investigates a topic',
  bound_target_count: 0,
};

const library: AgentPresetLibraryResponse = {
  official_templates: [
    {
      template_key: 'chat.minimal',
      seed: {
        initial_capabilities: [],
        on_demand_capabilities: [],
        skill_bindings: [],
        required_resource_kinds: [],
        required_runtime_features: [],
      },
      role_coverage: {
        required_capability_categories: [],
        required_capability_ids: [],
        required_runtime_features: [],
        required_resource_kinds: [],
      },
      immutable: true,
      forkable: true,
    },
  ],
  user_presets: [preset],
  active_bindings: [],
  fresh_start: {
    data_generation: 4,
    legacy_data_imported: false,
    official_template_count: 1,
    user_preset_count: 1,
  },
};

afterEach(() => cleanup());

describe('AgentPreset library deletion', () => {
  test('offers a confirmed delete trigger only for a user preset', () => {
    let selected = 0;
    const result = render(
      <I18nextProvider i18n={testI18n}>
        <AgentPresetLibrary
          library={library}
          selection={null}
          busy={false}
          creating={false}
          openingPresetId={null}
          deletingPresetId={null}
          onSelectTemplate={() => {}}
          onSelectPreset={() => {
            selected += 1;
          }}
          onCreatePreset={() => {}}
          onDeletePreset={() => {}}
        />
      </I18nextProvider>
    );
    const page = within(result.container);
    const body = within(document.body);

    const deleteTriggers = page.getAllByRole('button', {
      name: 'Delete Agent “Research Agent”',
    });
    expect(deleteTriggers).toHaveLength(1);
    expect(selected).toBe(0);
    expect(deleteTriggers[0].getAttribute('title')).toBe('Delete');
    expect(body.queryAllByRole('button', { name: 'Delete Agent “Minimal Chat”' })).toHaveLength(0);
  });

  test('shows the deleting state on the matching row', () => {
    const result = render(
      <I18nextProvider i18n={testI18n}>
        <AgentPresetLibrary
          library={library}
          selection={{ kind: 'preset', preset }}
          busy={true}
          creating={false}
          openingPresetId={null}
          deletingPresetId={preset.preset_id}
          onSelectTemplate={() => {}}
          onSelectPreset={() => {}}
          onCreatePreset={() => {}}
          onDeletePreset={() => {}}
        />
      </I18nextProvider>
    );
    const page = within(result.container);

    const deleteButton = page.getByRole('button', {
      name: 'Delete Agent “Research Agent”',
    });
    expect(deleteButton.hasAttribute('disabled')).toBe(true);
  });

});
