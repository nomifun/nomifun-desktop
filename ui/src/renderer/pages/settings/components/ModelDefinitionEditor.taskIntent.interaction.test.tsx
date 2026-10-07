import '../../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useMemo, useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import settings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import { purposeManifests } from '../../../../../test/fixtures/modelPurposeEditor';
import ModelDefinitionEditor, { type ModelCatalogSuggestion } from './ModelDefinitionEditor';
import { capabilityInputsFromDefinition, createModelDefinitionDraft, validateModelDefinition, withCatalogTaskEvidence } from './providerModelAdvanced';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings } } }, interpolation: { escapeValue: false } });
afterEach(cleanup);
const ENTRIES: ModelCatalogSuggestion[] = [
  { value: 'opaque-future', label: 'opaque-future', tasks: ['chat'], traits: [], tasksSource: 'inferred' },
  { value: 'opaque-asr', label: 'opaque-asr', tasks: ['speech_recognition'], traits: [], tasksSource: 'provider_declared' },
  { value: 'declared-chat', label: 'declared-chat', tasks: ['chat'], traits: [], tasksSource: 'provider_declared' },
];
function Harness({ task, entries = ENTRIES }: { task?: ModelTask; entries?: ModelCatalogSuggestion[] }) {
  const [value, setValue] = useState(() => createModelDefinitionDraft(task));
  const validation = useMemo(() => {
    const entry = entries.find((candidate) => candidate.value.trim() === value.model.trim());
    return validateModelDefinition(withCatalogTaskEvidence(value, entry ? { ...entry, model: entry.value } : undefined), purposeManifests, 'https://example.invalid/v1', [], [], [], 'bearer');
  }, [value, entries]);
  return <I18nextProvider i18n={i18n}><ModelDefinitionEditor value={value} onChange={setValue}
    providerBaseUrl='https://example.invalid/v1' providerAuthScheme='bearer' manifests={purposeManifests}
    validationErrors={validation.errors} catalogSuggestions={entries} />
    <button disabled={!validation.valid} data-testid='save'>保存模型</button>
    <output data-testid='tasks'>{value.capabilities.map((capability) => capability.task).join(',')}</output>
    <output data-testid='payload'>{JSON.stringify(capabilityInputsFromDefinition(value))}</output>
  </I18nextProvider>;
}
async function selectModel(screen: ReturnType<typeof render>, id: string) {
  fireEvent.click(screen.getByRole('button', { name: '查看模型列表' }));
  await waitFor(() => expect(screen.getByRole('option', { name: id })).toBeTruthy());
  fireEvent.click(screen.getByRole('option', { name: id }));
}

describe('model purpose confirmation and specialized entry intent', () => {
  test('unknown and inferred IDs stay unclassified until the user selects a purpose', async () => {
    const screen = render(<Harness />);
    fireEvent.change(screen.getByLabelText('模型 ID'), { target: { value: 'unknown-asr-id' } });
    expect(screen.getByTestId('tasks').textContent).toBe('');
    expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    await selectModel(screen, 'opaque-future');
    expect(screen.getByTestId('tasks').textContent).toBe('');
    fireEvent.click(screen.getByRole('combobox', { name: '调用用途' }));
    fireEvent.click(await screen.findByRole('option', { name: settings.modelTask.speech_recognition }));
    await waitFor(() => expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(false));
    expect(screen.getByTestId('tasks').textContent).toBe('speech_recognition');
    const payload = JSON.parse(screen.getByTestId('payload').textContent!);
    expect(payload[0]).toMatchObject({ task: 'speech_recognition', protocol: 'openai.audio_transcriptions' });
  });

  test('an ASR entry preserves its purpose when selecting an inferred catalog model or typing another ID', async () => {
    const screen = render(<Harness task='speech_recognition' />);
    await selectModel(screen, 'opaque-future');
    await waitFor(() => expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(false));
    expect(screen.getByTestId('tasks').textContent).toBe('speech_recognition');
    expect(screen.queryByRole('combobox', { name: '调用用途' })).toBeNull();
    fireEvent.change(screen.getByLabelText('模型 ID'), { target: { value: 'custom-audio-id' } });
    expect(screen.getByTestId('tasks').textContent).toBe('speech_recognition');
    expect(screen.getByTestId('payload').textContent).toContain('openai.audio_transcriptions');
  });

  test('verified native task metadata initializes ASR but a subsequent manual ID clears automatic purpose', async () => {
    const screen = render(<Harness />);
    await selectModel(screen, 'opaque-asr');
    await waitFor(() => expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(false));
    expect(screen.getByTestId('tasks').textContent).toBe('speech_recognition');
    fireEvent.change(screen.getByLabelText('模型 ID'), { target: { value: 'new-unknown-id' } });
    expect(screen.getByTestId('tasks').textContent).toBe('');
    expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole('combobox', { name: '调用用途' })).toBeTruthy();
  });

  test('a verified task conflict blocks saving until explicit confirmation, retaining the TTS interface', async () => {
    const screen = render(<Harness task='speech_synthesis' />);
    await selectModel(screen, 'declared-chat');
    expect(screen.getByTestId('tasks').textContent).toBe('speech_synthesis');
    expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '确认保留当前调用用途' }));
    await waitFor(() => expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(false));
    expect(screen.getByTestId('payload').textContent).toContain('openai.audio_speech');
    expect(screen.getByTestId('payload').textContent).not.toContain('openai.chat_text');
  });

  test('typing a known conflicting ID and editing away and back cannot bypass confirmation', async () => {
    const screen = render(<Harness task='speech_synthesis' />);
    const input = screen.getByLabelText('模型 ID');
    fireEvent.change(input, { target: { value: 'declared-chat' } });
    expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '确认保留当前调用用途' }));
    await waitFor(() => expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(false));
    fireEvent.change(input, { target: { value: 'declared-chat-x' } });
    fireEvent.change(input, { target: { value: 'declared-chat' } });
    expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole('button', { name: '确认保留当前调用用途' })).toBeTruthy();
    expect(screen.getByTestId('payload').textContent).toContain('openai.audio_speech');
  });

  test('verified task information arriving after manual input immediately blocks a mismatched purpose', async () => {
    const screen = render(<Harness task='speech_synthesis' entries={[]} />);
    fireEvent.change(screen.getByLabelText('模型 ID'), { target: { value: 'declared-chat' } });
    await waitFor(() => expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(false));
    screen.rerender(<Harness task='speech_synthesis' entries={ENTRIES} />);
    expect((screen.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    await waitFor(() => expect(screen.getByRole('button', { name: '确认保留当前调用用途' })).toBeTruthy());
    expect(screen.getByTestId('tasks').textContent).toBe('speech_synthesis');
  });
});
