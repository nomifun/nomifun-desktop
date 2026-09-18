import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type {
  AgentCatalogResponse,
  AgentPresetDocument,
  CapabilityModuleCatalogItem,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import { asCapabilityId, asPackageId } from '@/common/types/agentPlatform';
import OfficialTemplateOverview, { documentFromTemplate } from './OfficialTemplateOverview';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import common from '../../services/i18n/locales/en-US/common.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en, common } } },
});
afterEach(cleanup);

const knowledge: CapabilityModuleCatalogItem = {
  module: { id: asCapabilityId('knowledge'), version: '1.0.0' },
  display_name: 'Knowledge', description: 'Knowledge module',
  source_package: { id: asPackageId('nomifun.knowledge'), version: '1.0.0' },
  authoring_policy: 'direct', summary_kind: 'tool',
  actions: [
    { action_id: 'knowledge/search', input_schema: 'input', output_schema: 'output', effect_class: 'read_sensitive', presentation: 'function_tool' },
    { action_id: 'knowledge/write', input_schema: 'input', output_schema: 'output', effect_class: 'write_durable', presentation: 'function_tool' },
  ],
  context_schema_refs: [], event_schema_refs: [], required_resource_kinds: ['knowledge_base'],
  required_host_ports: [], required_modules: [], conflicting_modules: [], supported_surfaces: ['desktop'],
};
const catalog: AgentCatalogResponse = {
  modules: [knowledge],
  capabilities: [{
    capability: knowledge.module, kind: 'tool', display_name: 'Knowledge', description: 'Knowledge module',
    source_package: knowledge.source_package, source_kind: 'bundled', materialization_state: 'materialized',
    supported_surfaces: ['desktop'], required_runtime_features: [], required_resource_kinds: ['knowledge_base'],
    required_capabilities: [], conflicting_capabilities: [], action_count: 2, context_contributor_count: 0,
  }],
  skills: [], mcp_tools: [], roles: [],
};
const template: OfficialPresetTemplate = {
  template_key: 'assistant.general', immutable: true, forkable: true,
  seed: { enabled_capabilities: [{ capability: knowledge.module, action_allowlist: ['knowledge/search', 'knowledge/write'] }], skill_bindings: [], required_resource_kinds: ['knowledge_base'], required_runtime_features: [] },
  role_coverage: { required_capability_categories: [], required_capability_ids: [knowledge.module.id], required_runtime_features: [], required_resource_kinds: ['knowledge_base'] },
};

function mount(busy = false) {
  const saves: AgentPresetDocument[] = [];
  let dirty = false;
  const screen = render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter>
        <OfficialTemplateOverview
          template={template}
          busy={busy}
          catalog={catalog}
          onSave={(_name, document) => saves.push(structuredClone(document))}
          onDirtyChange={(value) => { dirty = value; }}
        />
      </MemoryRouter>
    </I18nextProvider>
  );
  return { ...screen, saves, dirty: () => dirty };
}

test('official template preserves exact server-seeded actions and never exposes a Runtime selector', () => {
  const screen = mount();
  expect(documentFromTemplate(template, catalog).enabled_capabilities[0]).toEqual({
    capability: knowledge.module,
    action_allowlist: ['knowledge/search', 'knowledge/write'],
  });
  expect(screen.getByRole('switch', { name: 'Disable Knowledge' })).toBeTruthy();
  expect(screen.queryByRole('combobox')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /2 actions and contributions/ }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Allow Write in Knowledge' }));
  fireEvent.click(screen.getByRole('button', { name: en.workbench.saveAsMine }));
  expect(screen.saves[0].enabled_capabilities[0].action_allowlist).toEqual(['knowledge/search']);
  expect(screen.saves[0]).not.toHaveProperty('runtime_engine');
});

test('template reset restores the exact server-seeded Module actions', async () => {
  const screen = mount();
  fireEvent.click(screen.getByRole('switch', { name: 'Disable Knowledge' }));
  await waitFor(() => expect(screen.dirty()).toBe(true));
  fireEvent.click(screen.getByRole('button', { name: en.workbench.resetTemplate }));
  await waitFor(() => expect(screen.dirty()).toBe(false));
  fireEvent.click(screen.getByRole('button', { name: en.workbench.saveAsMine }));
  expect(screen.saves[0]).toEqual(documentFromTemplate(template, catalog));
});

test('an action-bearing Module with no granted Action is visibly invalid and cannot save', () => {
  const screen = mount();
  fireEvent.click(screen.getByRole('button', { name: /2 actions and contributions/ }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Allow Search in Knowledge' }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Allow Write in Knowledge' }));
  expect(screen.getByText(en.workbench.actionRequired)).toBeTruthy();
  expect((screen.getByRole('button', { name: en.workbench.saveAsMine }) as HTMLButtonElement).disabled).toBe(true);
});

test('busy official template cannot change Modules or save', () => {
  const screen = mount(true);
  expect((screen.getByRole('switch', { name: 'Disable Knowledge' }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByRole('button', { name: en.workbench.saveAsMine }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.saves).toHaveLength(0);
});

test('an unavailable preselected Module remains visible and blocks save', () => {
  const unavailableCatalog: AgentCatalogResponse = {
    ...catalog,
    capabilities: [{
      ...catalog.capabilities[0],
      materialization_state: 'unavailable',
      unavailable_code: 'CAPABILITY_UNAVAILABLE_ON_PLATFORM',
    }],
  };
  const screen = render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter>
        <OfficialTemplateOverview
          template={template}
          busy={false}
          catalog={unavailableCatalog}
          onSave={() => { throw new Error('save must stay blocked'); }}
        />
      </MemoryRouter>
    </I18nextProvider>
  );
  expect(screen.getByText(en.workbench.disabledSave)).toBeTruthy();
  expect((screen.getByRole('button', { name: en.workbench.saveAsMine }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByRole('switch', { name: 'Disable Knowledge' })).toBeTruthy();
});
