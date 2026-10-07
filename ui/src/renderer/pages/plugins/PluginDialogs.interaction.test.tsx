import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type { ConfigurePluginRequest, PluginDetail, PluginImportInspection } from '@/common/types/pluginPlatform';
import PluginImportDialog from './PluginImportDialog';
import PluginConfigurationDialog from './PluginConfigurationDialog';
import en from '../../services/i18n/locales/en-US/pluginPlatform.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { pluginPlatform: en } } } });
const manifest = { schema: 'nomifun.plugin/v1', package_id: 'local.notes', version: '1.0.0', name: 'Local notes',
  description: 'A place for quick notes', host_api: '^1', entrypoints: { ui: 'index.html' }, actions: [], bindings: [],
  data_version: 1, config_schema: { type: 'object' }, secret_slots: [], permissions: [] };
const detail: PluginDetail = { summary: { plugin_id: 'notes', package_id: manifest.package_id, display_name: manifest.name,
  description: manifest.description, enabled: true, revision: 3,
  active: { artifact_digest: 'a'.repeat(64), package_version: '1.0.0', data_generation: 'notes-data', data_version: 1 },
  has_ui: true, has_service: false, action_count: 0, binding_count: 0, runtime: { state: 'stopped' }, updated_at_ms: 1 },
  manifest, config: { schema: manifest.config_schema, values: { accent: 'blue' }, valid: true, validation_errors: [] },
  credential_bindings: [], grants: [] };
const inspection: PluginImportInspection = { kind: 'zip', artifact_digest: 'a'.repeat(64), manifest };

beforeEach(() => {
  (window as typeof window & { __backendPort?: number }).__backendPort = 11451;
  spyOn(pluginPlatform.credentials.list, 'invoke').mockResolvedValue([]);
});
afterEach(() => { cleanup(); mock.restore(); delete (window as typeof window & { __backendPort?: number }).__backendPort; });

function importView(inspected = inspection) {
  spyOn(ipcBridge.dialog.showOpen, 'invoke').mockResolvedValue(['C:\\notes.zip']);
  spyOn(pluginPlatform.plugins.inspectImport, 'invoke').mockResolvedValue(inspected);
  spyOn(pluginPlatform.plugins.get, 'invoke').mockResolvedValue(detail);
  const install = spyOn(pluginPlatform.plugins.installImport, 'invoke').mockResolvedValue({ result: { outcome: 'installed', plugin: detail } });
  const installed = mock(() => {});
  const view = render(<I18nextProvider i18n={i18n}><PluginImportDialog visible onCancel={() => {}} onInstalled={installed} /></I18nextProvider>);
  return { ...view, install, installed };
}

test('a selected project imports in one step without extra approval and keeps technical settings collapsed', async () => {
  const v = importView();
  expect((v.getByRole('button', { name: en.import.install }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(v.getByRole('button', { name: new RegExp(en.import.zip) }));
  await v.findByText('Local notes');
  expect(v.queryByText('1.0.0')).toBeNull();
  expect((v.getByText(en.config.advanced).parentElement as HTMLDetailsElement).open).toBe(false);
  fireEvent.click(v.getByRole('button', { name: en.import.install }));
  await waitFor(() => expect(v.installed).toHaveBeenCalledWith(detail));
  expect(v.install).toHaveBeenCalledTimes(1);
  expect(v.install.mock.calls[0][0]).toEqual({ source_path: 'C:\\notes.zip', kind: 'zip', create_copy: false, config: {}, credential_bindings: {} });
});

test('returning from independent copy to update preserves the existing settings and concurrent update revision', async () => {
  const v = importView({ ...inspection, target_plugin_id: 'notes', target_plugin_revision: 3 });
  fireEvent.click(v.getByRole('button', { name: new RegExp(en.import.zip) }));
  await v.findByText('Local notes');
  const copy = v.getByRole('checkbox', { name: en.import.createCopy });
  fireEvent.click(copy); fireEvent.click(copy);
  fireEvent.click(v.getByRole('button', { name: en.import.install }));
  await waitFor(() => expect(v.installed).toHaveBeenCalled());
  expect(v.install.mock.calls[0][0]).toMatchObject({ expected_plugin_revision: 3, create_copy: false, config: { accent: 'blue' } });
});

test('saving unchanged settings preserves configuration and rejects invalid advanced edits', async () => {
  const submit = mock((_request: ConfigurePluginRequest) => {});
  const view = render(<I18nextProvider i18n={i18n}><PluginConfigurationDialog detail={detail} visible loading={false} onCancel={() => {}} onSubmit={submit} /></I18nextProvider>);
  fireEvent.click(view.getByRole('button', { name: en.actions.save }));
  await waitFor(() => expect(submit).toHaveBeenCalledWith({ expected_revision: 3, config: { accent: 'blue' }, credential_bindings: {}, grants: {} }));
  fireEvent.change(view.getByRole('textbox', { name: en.config.values }), { target: { value: '[' } });
  fireEvent.click(view.getByRole('button', { name: en.actions.save }));
  await view.findByText(en.config.invalidJson);
  expect(submit).toHaveBeenCalledTimes(1);
});

test('a settings write failure stays visible in the dialog and the same settings can be retried', async () => {
  const submit = mock(async (_request: ConfigurePluginRequest) => {}).mockRejectedValueOnce(new Error('Settings could not be saved'));
  const view = render(<I18nextProvider i18n={i18n}><PluginConfigurationDialog detail={detail} visible loading={false} onCancel={() => {}} onSubmit={submit} /></I18nextProvider>);
  fireEvent.click(view.getByRole('button', { name: en.actions.save }));
  await view.findByText('Settings could not be saved');
  expect(view.getByRole('dialog')).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: en.actions.save }));
  await waitFor(() => expect(submit).toHaveBeenCalledTimes(2));
  expect(submit.mock.calls[1][0]).toEqual(submit.mock.calls[0][0]);
});

test('simple settings expose typed controls and omit blank optional values from the saved configuration', async () => {
  const configSchema = { type: 'object', properties: {
    duration: { type: 'integer', title: 'Focus duration', default: 25 },
    ratio: { type: 'number', title: 'Ratio' }, notes: { type: 'string', title: 'Notes' },
    reminders: { type: 'boolean', title: 'Enable reminders', default: false }, mode: { type: 'integer', title: 'Mode', enum: [1, 2], default: 1 },
  }, required: ['duration'], additionalProperties: false };
  const configured: PluginDetail = { ...detail, manifest: { ...detail.manifest, config_schema: configSchema },
    config: { ...detail.config, values: { notes: 'Temporary' } } };
  const submit = mock((_request: ConfigurePluginRequest) => {});
  const view = render(<I18nextProvider i18n={i18n}><PluginConfigurationDialog detail={configured} visible loading={false} onCancel={() => {}} onSubmit={submit} /></I18nextProvider>);
  fireEvent.change(view.getByLabelText('Focus duration'), { target: { value: '40' } });
  fireEvent.blur(view.getByLabelText('Focus duration'));
  fireEvent.change(view.getByLabelText('Ratio'), { target: { value: '0' } });
  fireEvent.blur(view.getByLabelText('Ratio'));
  fireEvent.change(view.getByRole('textbox', { name: 'Notes' }), { target: { value: '' } });
  fireEvent.click(view.getByRole('checkbox', { name: 'Enable reminders' }));
  fireEvent.click(view.getByLabelText('Mode'));
  fireEvent.click(await view.findByText('2'));
  fireEvent.click(view.getByRole('button', { name: en.actions.save }));
  await waitFor(() => expect(submit).toHaveBeenCalledTimes(1));
  expect(submit.mock.calls[0][0].config).toEqual({ duration: 40, ratio: 0, reminders: true, mode: 2 });
  expect((view.getByText(en.config.editJson).parentElement as HTMLDetailsElement).open).toBe(false);
});

test('JSON edits update the settings fields and subsequent field edits preserve other JSON data', async () => {
  const configSchema = { type: 'object', properties: { title: { type: 'string', title: 'Display name' }, count: { type: 'integer', title: 'Count' } } };
  const configured: PluginDetail = { ...detail, manifest: { ...detail.manifest, config_schema: configSchema }, config: { ...detail.config, values: { title: 'Original', count: 1 } } };
  const submit = mock((_request: ConfigurePluginRequest) => {});
  const view = render(<I18nextProvider i18n={i18n}><PluginConfigurationDialog detail={configured} visible loading={false} onCancel={() => {}} onSubmit={submit} /></I18nextProvider>);
  fireEvent.click(view.getByText(en.config.editJson));
  fireEvent.change(view.getByRole('textbox', { name: en.config.values }), { target: { value: '{"title":"From JSON","count":9,"other":{"retained":true}}' } });
  expect((view.getByRole('textbox', { name: 'Display name' }) as HTMLInputElement).value).toBe('From JSON');
  expect((view.getByLabelText('Count') as HTMLInputElement).value).toBe('9');
  fireEvent.change(view.getByRole('textbox', { name: 'Display name' }), { target: { value: 'From field' } });
  fireEvent.click(view.getByRole('button', { name: en.actions.save }));
  await waitFor(() => expect(submit).toHaveBeenCalledTimes(1));
  expect(submit.mock.calls[0][0].config).toEqual({ title: 'From field', count: 9, other: { retained: true } });
});

test('complex required settings keep the full schema and JSON editor visible', async () => {
  const configSchema = { type: 'object', description: 'Choose an account and its connection details.', properties: {
    account: { type: 'object', properties: { token: { type: 'string' } }, required: ['token'] },
  }, required: ['account'] };
  const configured: PluginDetail = { ...detail, manifest: { ...detail.manifest, config_schema: configSchema }, config: { ...detail.config, values: { account: { token: 'reference' } } } };
  const submit = mock((_request: ConfigurePluginRequest) => {});
  const view = render(<I18nextProvider i18n={i18n}><PluginConfigurationDialog detail={configured} visible loading={false} onCancel={() => {}} onSubmit={submit} /></I18nextProvider>);
  expect(view.getByText(configSchema.description)).toBeTruthy();
  expect(view.queryByText(en.config.emptyTitle)).toBeNull();
  expect((view.getByText(en.config.editJson).parentElement as HTMLDetailsElement).open).toBe(true);
  expect(JSON.parse((view.getByRole('textbox', { name: en.config.values }) as HTMLTextAreaElement).value)).toEqual({ account: { token: 'reference' } });
  fireEvent.click(view.getByText(en.config.schema));
  expect(view.baseElement.querySelector('pre')?.textContent ?? '').toContain('"required"');
  fireEvent.click(view.getByRole('button', { name: en.actions.save }));
  await waitFor(() => expect(submit).toHaveBeenCalledTimes(1));
  expect(submit.mock.calls[0][0].config).toEqual({ account: { token: 'reference' } });
});
