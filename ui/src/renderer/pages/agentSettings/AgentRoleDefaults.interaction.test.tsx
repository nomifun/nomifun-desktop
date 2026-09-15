import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import { asCapabilityId, asDigestHex, asPackageId, type AgentCatalogResponse, type RoleProviderSelection } from '@/common/types/agentPlatform';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import AgentRoleDefaults from './AgentRoleDefaults';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en } } }, interpolation: { escapeValue: false } });
const role = { key: { role_id: 'test.context', contract_version: '1.0.0' }, contract_digest: asDigestHex('a'.repeat(64)) };
const a: RoleProviderSelection = { role, provider_mount_id: 'provider-a' };
const b: RoleProviderSelection = { role, provider_mount_id: 'provider-b' };
const capability = { id: asCapabilityId('test.context.facade'), version: '1.0.0' };
const catalog: AgentCatalogResponse = { capabilities: [], skills: [], mcp_tools: [], roles: [{ role, capabilities: [capability],
  providers: [a, b].map((selection, index) => ({ selection, display_name: `Provider ${index}`, description: '',
    source_package: { id: asPackageId('test.plugin'), version: '1.0.0' }, source_kind: 'managed_local', supported_capabilities: [capability] })) }] };

afterEach(() => { cleanup(); mock.restore(); });

async function open(current = catalog) {
  const result = render(<I18nextProvider i18n={i18n}><AgentRoleDefaults catalog={current} /></I18nextProvider>);
  await act(async () => { fireEvent.click(within(result.container).getByRole('button', { name: en.providers.defaultsTitle })); });
  return within(document.body);
}

test('default selection is explicit and saves the exact Catalog identity with create CAS', async () => {
  spyOn(agentPlatform.roleDefaults, 'invoke').mockResolvedValue([]);
  const save = spyOn(agentPlatform.putRoleDefault, 'invoke').mockResolvedValue({ selection: b, binding_version: 1, updated_at_ms: 10 });
  const view = await open();
  fireEvent.click(await view.findByRole('combobox', { name: capability.id }));
  fireEvent.click(await view.findByText('Provider 1 — test.plugin@1.0.0'));
  expect(save).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button', { name: en.providers.defaultsSave }));
  await waitFor(() => expect(save).toHaveBeenCalledWith({ selection: b, expected_binding_version: 0 }));
  await waitFor(() => expect((view.getByRole('button', { name: en.providers.defaultsSave }) as HTMLButtonElement).disabled).toBe(true));
});

test('a stale write reports failure and never overwrites with an automatic retry', async () => {
  spyOn(agentPlatform.roleDefaults, 'invoke').mockResolvedValue([{ selection: a, binding_version: 7, updated_at_ms: 10 }]);
  const save = spyOn(agentPlatform.putRoleDefault, 'invoke').mockRejectedValue(new Error('ROLE_DEFAULT_VERSION_CONFLICT'));
  const view = await open();
  fireEvent.click(await view.findByRole('combobox', { name: capability.id }));
  fireEvent.click(await view.findByText('Provider 1 — test.plugin@1.0.0'));
  fireEvent.click(view.getByRole('button', { name: en.providers.defaultsSave }));
  await view.findByText(en.providers.defaultsSaveError);
  expect(save).toHaveBeenCalledTimes(1);
  expect(save).toHaveBeenCalledWith({ selection: b, expected_binding_version: 7 });
});

test('a withdrawn default is retained and is not replaced with an available Provider', async () => {
  spyOn(agentPlatform.roleDefaults, 'invoke').mockResolvedValue([{ selection: a, binding_version: 3, updated_at_ms: 10 }]);
  const save = spyOn(agentPlatform.putRoleDefault, 'invoke');
  const view = await open({ ...catalog, roles: [] });
  await view.findByText(en.providers.defaultsMissingHint);
  expect((view.getByRole('button', { name: en.providers.defaultsSave }) as HTMLButtonElement).disabled).toBe(true);
  expect(save).not.toHaveBeenCalled();
});

test('an owner/read failure does not manufacture an empty binding to write', async () => {
  spyOn(agentPlatform.roleDefaults, 'invoke').mockRejectedValue(new Error('forbidden'));
  const save = spyOn(agentPlatform.putRoleDefault, 'invoke');
  const view = await open();
  await view.findByText(en.providers.defaultsLoadError);
  expect(view.queryByRole('combobox')).toBeNull();
  expect(save).not.toHaveBeenCalled();
});
