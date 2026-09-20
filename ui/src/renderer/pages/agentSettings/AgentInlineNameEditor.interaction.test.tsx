import '../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import AgentInlineNameEditor from './AgentInlineNameEditor';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: { agentSettings: en } } },
  interpolation: { escapeValue: false },
});

afterEach(cleanup);

const mount = () => render(<I18nextProvider i18n={i18n}>
  <Harness />
</I18nextProvider>);

const Harness = () => {
  const [name, setName] = useState('Original Agent');
  return <AgentInlineNameEditor value={name} fallback='Untitled Agent' onChange={setName} />;
};

test('clicking the header name supports blur commit and Escape cancellation', () => {
  const screen = mount();
  fireEvent.click(screen.getByRole('heading', { name: 'Original Agent' }));
  const first = screen.getByRole('textbox', { name: en.fields.name });
  fireEvent.input(first, { target: { value: 'Renamed Agent' } });
  fireEvent.blur(first);
  expect(screen.getByRole('heading', { name: 'Renamed Agent' })).toBeTruthy();

  fireEvent.click(screen.getByRole('button', { name: en.workbench.editName }));
  const second = screen.getByRole('textbox', { name: en.fields.name });
  fireEvent.input(second, { target: { value: 'Discarded name' } });
  fireEvent.keyDown(second, { key: 'Escape' });
  expect(screen.getByRole('heading', { name: 'Renamed Agent' })).toBeTruthy();

  fireEvent.click(screen.getByRole('heading', { name: 'Renamed Agent' }));
  const third = screen.getByRole('textbox', { name: en.fields.name });
  fireEvent.input(third, { target: { value: 'Final Agent' } });
  fireEvent.blur(third);
  expect(screen.getByRole('heading', { name: 'Final Agent' })).toBeTruthy();
});
