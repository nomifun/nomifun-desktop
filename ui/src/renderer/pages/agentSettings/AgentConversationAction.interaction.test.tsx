/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import { AgentConversationAction } from './AgentPresetEditor';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {
        agentSettings: {
          actions: {
            startConversation: 'Start conversation',
          },
        },
      },
    },
  },
  interpolation: { escapeValue: false },
});

afterEach(() => cleanup());

describe('Agent conversation action', () => {
  const renderAction = (
    props: Pick<
      React.ComponentProps<typeof AgentConversationAction>,
      'hasStableRevision' | 'dirty' | 'onClick'
    >
  ) => {
    const result = render(
      <I18nextProvider i18n={testI18n}>
        <AgentConversationAction {...props} />
      </I18nextProvider>
    );
    return within(result.container);
  };

  test('is disabled when the preset has no saved stable revision', () => {
    let calls = 0;
    const page = renderAction({
      hasStableRevision: false,
      dirty: false,
      onClick: () => calls += 1,
    });

    const button = page.getByRole('button', { name: 'Start conversation' });
    expect(button.hasAttribute('disabled')).toBe(true);
    fireEvent.click(button);
    expect(calls).toBe(0);
  });

  test('is disabled when the saved preset has dirty draft changes', () => {
    let calls = 0;
    const page = renderAction({
      hasStableRevision: true,
      dirty: true,
      onClick: () => calls += 1,
    });

    const button = page.getByRole('button', { name: 'Start conversation' });
    expect(button.hasAttribute('disabled')).toBe(true);
    fireEvent.click(button);
    expect(calls).toBe(0);
  });

  test('invokes the action once when the saved draft is clean', () => {
    let calls = 0;
    const page = renderAction({
      hasStableRevision: true,
      dirty: false,
      onClick: () => calls += 1,
    });

    const button = page.getByRole('button', { name: 'Start conversation' });
    expect(button.hasAttribute('disabled')).toBe(false);
    fireEvent.click(button);
    expect(calls).toBe(1);
  });
});
