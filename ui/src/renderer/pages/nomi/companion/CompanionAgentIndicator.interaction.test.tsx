import '../../../../../test/setup-dom.ts';

import { cleanup, render } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import CompanionAgentIndicator from './CompanionAgentIndicator';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en',
  keySeparator: false,
  resources: { en: { translation: {
    'agentSettings.template.companion.default.name': 'Companion',
    'nomi.chat.fixedAgentAria': 'Fixed Agent: {{agent}}',
    'nomi.chat.fixedAgentHint': 'Companion conversations always use the official Companion Agent.',
  } } },
});

afterEach(cleanup);

test('shows the fixed Companion Agent as identity rather than a selector', () => {
  const view = render(
    <I18nextProvider i18n={i18n}>
      <CompanionAgentIndicator />
    </I18nextProvider>
  );

  const indicator = view.getByTestId('companion-agent-indicator');
  expect(indicator.textContent).toBe('Companion');
  expect(indicator.getAttribute('aria-label')).toBe('Fixed Agent: Companion');
  expect(indicator.getAttribute('data-readonly')).toBe('true');
  expect(view.queryByRole('button')).toBeNull();
});
