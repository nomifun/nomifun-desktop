/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import type {
  ExecutableAgentPreset,
  GuidAgentSelection,
} from '../types';
import AgentPillBar from './AgentPillBar';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {
        agentSettings: {
          navigation: {
            railTitle: 'Agent Workbench',
          },
        },
        guid: {
          defaultAgent: 'Nomi Agent',
        },
      },
    },
  },
  interpolation: { escapeValue: false },
});

const preset = (
  presetId: string,
  displayName: string
): ExecutableAgentPreset =>
  ({
    preset_id: presetId,
    source: 'user',
    display_name: displayName,
    bound_target_count: 0,
    current_stable_revision: {
      preset_id: presetId,
      revision: 1,
      revision_digest: 'a'.repeat(64),
    },
  }) as ExecutableAgentPreset;

const LocationProbe = () => {
  const location = useLocation();
  return <output data-testid='location'>{location.pathname}</output>;
};

const renderBar = (
  presets: ExecutableAgentPreset[],
  selection: GuidAgentSelection = { kind: 'default' }
) => {
  const selections: GuidAgentSelection[] = [];

  const page = render(
    <I18nextProvider i18n={testI18n}>
      <MemoryRouter initialEntries={['/guid']}>
        <AgentPillBar
          presets={presets}
          selection={selection}
          onSelectPreset={(presetId) =>
            selections.push({ kind: 'preset', presetId })
          }
          onSelectDefault={() => selections.push({ kind: 'default' })}
        />
        <LocationProbe />
      </MemoryRouter>
    </I18nextProvider>
  );

  return { page, selections };
};

afterEach(() => cleanup());

describe('AgentPillBar launch modes', () => {
  test('renders default Nomi together with saved executable presets', () => {
    const first = preset(
      '0190f5fe-7c00-7a00-8000-000000000101',
      'Release reviewer'
    );
    const second = preset(
      '0190f5fe-7c00-7a00-8000-000000000102',
      'Research assistant'
    );
    const { page } = renderBar([first, second]);

    expect(
      page.getByRole('button', { name: 'Nomi Agent' })
    ).not.toBeNull();
    expect(
      page.getByRole('button', { name: 'Release reviewer' })
    ).not.toBeNull();
    expect(
      page.getByRole('button', { name: 'Research assistant' })
    ).not.toBeNull();
  });

  test('switches between default Nomi and a preset selection', () => {
    const savedPreset = preset(
      '0190f5fe-7c00-7a00-8000-000000000102',
      'Research assistant'
    );
    const { page, selections } = renderBar([savedPreset]);
    expect(
      page
        .getByRole('button', { name: 'Nomi Agent' })
        .getAttribute('aria-pressed')
    ).toBe('true');

    fireEvent.click(
      page.getByRole('button', { name: 'Research assistant' })
    );
    expect(
      selections.some(
        (selection) =>
          selection.kind === 'preset' &&
          selection.presetId === savedPreset.preset_id
      )
    ).toBe(true);

    fireEvent.click(page.getByRole('button', { name: 'Nomi Agent' }));
    expect(
      selections.some((selection) => selection.kind === 'default')
    ).toBe(true);
  });

  test('marks a workbench-preselected preset as the active mode', () => {
    const savedPreset = preset(
      '0190f5fe-7c00-7a00-8000-000000000103',
      'Code reviewer'
    );
    const { page } = renderBar([savedPreset], {
      kind: 'preset',
      presetId: savedPreset.preset_id,
    });

    expect(
      page
        .getByRole('button', { name: 'Code reviewer' })
        .getAttribute('aria-pressed')
    ).toBe('true');
    expect(
      page
        .getByRole('button', { name: 'Nomi Agent' })
        .getAttribute('aria-pressed')
    ).toBe('false');
  });

  test('keeps default Nomi and the workbench CTA when no presets exist', () => {
    const { page } = renderBar([]);

    expect(
      page.getByRole('button', { name: 'Nomi Agent' })
    ).not.toBeNull();
    expect(page.container.querySelector('[data-agent-preset-id]')).toBeNull();

    fireEvent.click(
      page.getByRole('button', { name: 'Agent Workbench' })
    );
    expect(page.getByTestId('location').textContent).toBe('/agent');
  });
});
