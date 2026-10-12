import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { ipcBridge } from '@/common';
import type { TProviderWithModel } from '@/common/config/storage';
import { parseExecutionTemplateId, parseProviderId } from '@/common/types/ids';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { uuidv7 } from '@/common/utils';
import * as catalog from '@/renderer/pages/conversation/execution/useExecutionModelPool';
import { SessionCapabilityComposerLayout } from '@/renderer/components/chat/SessionCapabilityPicker/ComposerLayout';
import enCollaboration from '@/renderer/services/i18n/locales/en-US/collaboration.json';
import CollaborationComposerControl from './CollaborationComposerControl';
import { useCollaborationDraft } from './useCollaborationDraft';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {
  collaboration: enCollaboration,
  guid: { collaboration: { models: { title: 'Models', mainTag: 'lead', emptyHint: 'The lead always participates.' } } },
} } } });
const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000931');
const main = { provider_id: providerId, model: 'Lead' };
const helper = { provider_id: providerId, model: 'Helper' };
const model = { id: providerId, use_model: main.model } as TProviderWithModel;
const templateId = parseExecutionTemplateId('0190f5fe-7c00-7a00-8000-000000000932');
const plan = { execution_template_id: templateId, name: 'Review plan', version: 1,
  participants: [main, helper] } as any;

beforeEach(() => {
  setBrowserStorageGeneration(uuidv7());
  spyOn(catalog, 'useExecutionModelPool').mockReturnValue({ providers: [{ id: providerId, name: 'Provider' } as any],
    getAvailableModels: () => [main.model, helper.model], formatModelLabel: (_provider, name) => name ?? '',
    isLoading: false, configuredPairs: [main, helper], allPairs: [main, helper], hasModels: true, buildModelPool: () => null });
  spyOn(ipcBridge.agentExecutionTemplate.list, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.agentExecutionTemplate.get, 'invoke').mockResolvedValue(plan);
  spyOn(ipcBridge.agentExecutionTemplate.create, 'invoke').mockResolvedValue(plan);
});
afterEach(() => { cleanup(); mock.restore(); sessionStorage.clear(); });

function View({ disabled = false, note = 'New session · applies on creation' }: { disabled?: boolean; note?: string }) {
  const draft = useCollaborationDraft(model);
  const picker = <aside data-composer-tools><CollaborationComposerControl value={draft.activeCollaborators}
    onChange={draft.setCollaborators} mainModel={draft.mainModel} selectedTemplate={draft.selectedTemplate}
    onTemplateApply={draft.setTemplate} onTemplateClear={() => draft.setTemplate(null)} policy={draft.policy}
    onPolicyChange={draft.setPolicy} disabled={disabled} disabledReason={disabled ? 'Finish the active turn to edit.' : undefined} applyNote={note} /></aside>;
  return <MemoryRouter><I18nextProvider i18n={i18n}><div data-composer-surface>
    <SessionCapabilityComposerLayout picker={picker}><textarea aria-label='Message' /></SessionCapabilityComposerLayout>
    <output data-testid='draft'>{JSON.stringify(draft.config)}</output>
  </div></I18nextProvider></MemoryRouter>;
}

test.each(['New session · applies on creation', 'This session · applies on next send'])('both entry points edit models and policies in the same compact panel (%s)', async note => {
  const view = render(<View note={note} />);
  fireEvent.click(view.getByTestId('guid-collaborator-selector'));
  await view.findByTestId('collaboration-composer-panel');
  expect(view.getByText(note)).toBeTruthy();
  expect(await view.findByText('No saved plans')).toBeTruthy();
  fireEvent.click(view.getByRole('radio', { name: 'Prefer parallel' }));
  fireEvent.click(view.getByRole('switch', { name: 'Ask when unsure' }));
  fireEvent.click(view.getByRole('combobox'));
  fireEvent.click(within(await view.findByRole('option', { name: 'Helper' })).getByText('Helper'));
  await waitFor(() => expect(JSON.parse(view.getByTestId('draft').textContent!)).toMatchObject({
    execution_model_pool: { mode: 'range', models: [main, helper] }, delegation_policy: 'prefer_parallel', decision_policy: 'ask_user',
  }));
});

test('plan saving, applying and clearing keep their existing business payloads', async () => {
  const create = spyOn(ipcBridge.agentExecutionTemplate.create, 'invoke').mockResolvedValue(plan);
  const list = spyOn(ipcBridge.agentExecutionTemplate.list, 'invoke').mockResolvedValue([plan]);
  const get = spyOn(ipcBridge.agentExecutionTemplate.get, 'invoke').mockResolvedValue(plan);
  const view = render(<View />);
  fireEvent.click(view.getByTestId('guid-collaborator-selector'));
  fireEvent.change(await view.findByPlaceholderText('Save the current setup as a plan'), { target: { value: 'My plan' } });
  fireEvent.click(view.getByRole('button', { name: 'Save' }));
  await waitFor(() => expect(create).toHaveBeenCalledTimes(1));
  expect(create.mock.calls[0][0]).toMatchObject({ name: 'My plan', max_parallel: 1,
    participants: [{ provider_id: providerId, model: main.model, sort_order: 0 }] });
  fireEvent.click(await view.findByRole('button', { name: 'Stop using' }));
  expect(JSON.parse(view.getByTestId('draft').textContent!).execution_template_id).toBeNull();
  fireEvent.click(await view.findByRole('button', { name: 'Review plan' }));
  await waitFor(() => expect(get).toHaveBeenCalledWith({ execution_template_id: templateId }));
  await waitFor(() => expect(JSON.parse(view.getByTestId('draft').textContent!).execution_template_id).toBe(templateId));
  expect(list).toHaveBeenCalled();
  expect(view.getByRole('button', { name: 'Delete plan' })).toBeTruthy();
});

test('busy conversations retain an openable icon while every mutation control is locked', async () => {
  const view = render(<View disabled />);
  const trigger = view.getByTestId('guid-collaborator-selector') as HTMLButtonElement;
  expect(trigger.disabled).toBe(false);
  fireEvent.click(trigger);
  await view.findByTestId('collaboration-composer-panel');
  expect(view.getByText('Finish the active turn to edit.')).toBeTruthy();
  expect(view.getByRole('combobox').getAttribute('aria-disabled')).toBe('true');
  expect((view.getByRole('radio', { name: 'Prefer parallel' }) as HTMLButtonElement).disabled).toBe(true);
  expect((view.getByRole('switch', { name: 'Ask when unsure' }) as HTMLButtonElement).disabled).toBe(true);
  expect((view.getByPlaceholderText('Save the current setup as a plan') as HTMLInputElement).disabled).toBe(true);
  expect((view.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(true);
});
