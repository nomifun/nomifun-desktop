import '../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import en from '@/renderer/services/i18n/locales/en-US/pluginWorkbench.json';
import type { PluginProductItem } from './pluginProductModel';
import PluginProductHome from './PluginProductHome';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: { pluginWorkbench: en } } },
  interpolation: { escapeValue: false },
});

const items: PluginProductItem[] = [
  {
    key: 'knowledge',
    displayName: 'Web summary',
    description: 'Summarize web content',
    category: 'knowledge',
    status: 'enabled',
    contributionCount: 2,
    updatedAtMs: 2,
  },
  {
    key: 'development',
    displayName: 'Code review',
    description: 'Review repository changes',
    category: 'development',
    status: 'draft',
    contributionCount: 0,
    updatedAtMs: 1,
  },
];

afterEach(() => cleanup());

describe('Plugin product home', () => {
  test('starts AI creation directly from a natural-language requirement', async () => {
    let submitted = '';
    const page = render(
      <I18nextProvider i18n={testI18n}>
        <PluginProductHome
          items={items}
          loading={false}
          search=''
          onSearch={() => {}}
          onCreate={(value) => { submitted = value; }}
          onImport={() => {}}
          onOpen={() => {}}
          onToggleEnabled={() => {}}
        />
      </I18nextProvider>
    );
    const requirement = page.getByLabelText(
      'Describe what you want NomiFun to do better…'
    ) as HTMLTextAreaElement;
    fireEvent.input(requirement, {
      target: { value: 'Add a structured research summary' },
    });
    await waitFor(() =>
      expect(requirement.value).toBe('Add a structured research summary')
    );
    const generate = page.getByRole('button', { name: 'Generate with AI' });
    await waitFor(() => expect((generate as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(generate);
    await waitFor(() => expect(submitted).toBe('Add a structured research summary'));
  });

  test('filters a large library by product capability category', () => {
    const page = render(
      <I18nextProvider i18n={testI18n}>
        <PluginProductHome
          items={items}
          loading={false}
          search=''
          onSearch={() => {}}
          onCreate={() => {}}
          onImport={() => {}}
          onOpen={() => {}}
          onToggleEnabled={() => {}}
        />
      </I18nextProvider>
    );
    fireEvent.click(page.getAllByText('Development')[0]!.closest('button')!);
    expect(page.queryByText('Web summary')).toBeNull();
    expect(page.getByText('Code review')).toBeTruthy();
  });
});
