import '../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { configService } from '@/common/config/configService';
import type {
  AgentPresetLibraryResponse,
  AgentPresetSummary,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import agentSettings from '@/renderer/services/i18n/locales/en-US/agentSettings.json';
import GuidDefaultAgentSetting from './GuidDefaultAgentSetting';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: { common, agentSettings } } },
  interpolation: { escapeValue: false },
});

const stablePreset = {
  preset_id: '0190f5fe-7c00-7a00-8000-000000000101',
  source: 'user',
  display_name: 'Release reviewer',
  bound_target_count: 0,
  current_stable_revision: {
    preset_id: '0190f5fe-7c00-7a00-8000-000000000101',
    revision: 1,
    revision_digest: 'a'.repeat(64),
  },
} as AgentPresetSummary;
const draftPreset = {
  ...stablePreset,
  preset_id: '0190f5fe-7c00-7a00-8000-000000000102',
  display_name: 'Draft Agent',
  current_stable_revision: undefined,
} as AgentPresetSummary;
const productPreset = {
  ...stablePreset,
  preset_id: '0190f5fe-7c00-7a00-8000-000000000103',
  display_name: 'Bound support Agent',
} as AgentPresetSummary;

const templates = [
  'chat.minimal',
  'assistant.general',
  'coding.codex',
  'companion.default',
  'customer-service.default',
].map((template_key) => ({ template_key })) as OfficialPresetTemplate[];

const library = {
  official_templates: templates,
  user_presets: [stablePreset, draftPreset, productPreset],
  active_bindings: [{
    target_kind: 'customer',
    preset_revision_ref: {
      preset_id: productPreset.preset_id,
      revision: 1,
    },
  }],
  fresh_start: {
    data_generation: 1,
    legacy_data_imported: false,
    official_template_count: templates.length,
    user_preset_count: 3,
  },
} as AgentPresetLibraryResponse;

beforeEach(() => configService.reset());
afterEach(() => {
  cleanup();
  configService.reset();
});

describe('Guid default Agent workbench setting', () => {
  test('shows the configured default and saves a new catalog-backed choice', async () => {
    configService.setLocal('guid.defaultAgentSelection', {
      kind: 'preset',
      presetId: stablePreset.preset_id,
    });
    const save = spyOn(configService, 'set').mockResolvedValue(undefined);
    try {
      const page = render(
        <I18nextProvider i18n={i18n}>
          <GuidDefaultAgentSetting library={library} />
        </I18nextProvider>
      );

      fireEvent.click(page.getByRole('button', {
        name: `Default Agent: ${stablePreset.display_name}`,
      }));
      const select = page.getByRole('combobox', {
        name: agentSettings.defaultAgent.field,
      }) as HTMLSelectElement;
      expect([...select.options].map((option) => option.text)).toContain(
        agentSettings.template.coding.codex.name
      );
      expect([...select.options].map((option) => option.text)).toContain(
        stablePreset.display_name
      );
      expect([...select.options].map((option) => option.text)).not.toContain(
        draftPreset.display_name
      );
      expect([...select.options].map((option) => option.text)).not.toContain(
        productPreset.display_name
      );
      expect([...select.options].map((option) => option.text)).not.toContain(
        agentSettings.template.companion.default.name
      );

      fireEvent.change(select, { target: { value: 'template:coding.codex' } });
      await act(async () => {
        fireEvent.click(page.getByRole('button', {
          name: agentSettings.defaultAgent.save,
        }));
      });
      expect(save).toHaveBeenCalledWith('guid.defaultAgentSelection', {
        kind: 'template',
        templateKey: 'coding.codex',
      });
    } finally {
      save.mockRestore();
    }
  });

  test('uses the legacy Guid selection only as an upgrade fallback', () => {
    configService.setLocal('guid.agentSelection', {
      kind: 'template',
      templateKey: 'chat.minimal',
    });
    const page = render(
      <I18nextProvider i18n={i18n}>
        <GuidDefaultAgentSetting library={library} />
      </I18nextProvider>
    );

    expect(page.getByRole('button', {
      name: `Default Agent: ${agentSettings.template.chat.minimal.name}`,
    })).not.toBeNull();
  });
});
