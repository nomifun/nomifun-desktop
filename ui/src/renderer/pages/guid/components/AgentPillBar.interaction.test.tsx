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
import type { ExecutableAgentPreset } from '../types';
import AgentPillBar from './AgentPillBar';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {
        settings: {
          agentManagement: {
            title: 'Agent Workbench',
          },
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
  onSelectPreset: (presetId: string) => void = () => undefined
) =>
  render(
    <I18nextProvider i18n={testI18n}>
      <MemoryRouter initialEntries={['/guid']}>
        <AgentPillBar
          presets={presets}
          selectedPresetId={presets[0]?.preset_id ?? ''}
          onSelectPreset={onSelectPreset}
        />
        <LocationProbe />
      </MemoryRouter>
    </I18nextProvider>
  );

afterEach(() => cleanup());

describe('AgentPillBar preset behavior', () => {
  test('selects a rendered preset by preset_id', () => {
    const first = preset(
      '0190f5fe-7c00-7a00-8000-000000000101',
      'Release reviewer'
    );
    const second = preset(
      '0190f5fe-7c00-7a00-8000-000000000102',
      'Research assistant'
    );
    const selected: string[] = [];
    const page = renderBar([first, second], (presetId) => selected.push(presetId));

    expect(page.getByText('Release reviewer').textContent).toBe(
      'Release reviewer'
    );
    expect(page.getByText('Research assistant').textContent).toBe(
      'Research assistant'
    );
    expect(
      page
        .getByTestId(`agent-pill-${second.preset_id}`)
        .className.includes('max-w-0')
    ).toBe(false);
    expect(
      page.getByText('Research assistant').className.includes('opacity-0')
    ).toBe(false);
    fireEvent.click(page.getByTestId(`agent-pill-${second.preset_id}`));

    expect(selected).toEqual([second.preset_id]);
  });

  test('keeps the workbench plus CTA when no executable presets exist', () => {
    const page = renderBar([]);

    expect(page.container.querySelector('[data-agent-pill]')).toBeNull();
    const plusIcon = page.container.querySelector('.i-icon-plus');
    expect(plusIcon).not.toBeNull();
    fireEvent.click(plusIcon as Element);
    expect(page.getByTestId('location').textContent).toBe('/agent');
  });
});
