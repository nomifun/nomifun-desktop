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
import {
  asCapabilityId,
  asPackageId,
  type CapabilityCatalogItem,
  type CapabilityPlacement,
} from '@/common/types/agentPlatform';
import AgentCapabilityList from './AgentCapabilityList';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {
        agentSettings: {
          common: {
            available: 'Available',
            unavailable: 'Unavailable',
          },
          capabilities: {
            source: 'Source',
            tools: '{{count}} actions',
            contexts: '{{count}} context contributors',
            catalogUnavailable: 'No implementation is available.',
            modeAria: 'Set the capability mode for {{name}}',
            notSelected: 'Off',
            initialShort: 'At startup',
            onDemandShort: 'May request',
            emptySelection: 'No capabilities.',
          },
          resources: {
            requiredAtUse: 'Required when used',
            noneRequired: 'No additional resources',
            unavailable: 'Not available',
            kinds: {
              knowledgeBase: 'Knowledge base',
            },
          },
        },
      },
    },
  },
  interpolation: { escapeValue: false },
});

const catalogItem = (
  id = 'knowledge.search',
  displayName = 'Search knowledge'
): CapabilityCatalogItem => ({
  capability: {
    id: asCapabilityId(id),
    version: '1.0.0',
  },
  kind: 'tool',
  display_name: displayName,
  description:
    id === 'knowledge.search'
      ? 'Searches knowledge selected by the current conversation.'
      : `${displayName} description.`,
  source_package: {
    id: asPackageId('nomifun.knowledge'),
    version: '1.0.0',
  },
  source_kind: 'first_party',
  materialization_state: 'materialized',
  supported_surfaces: ['desktop'],
  required_runtime_features: [],
  required_resource_kinds: ['knowledge_base'],
  required_capabilities: [],
  conflicting_capabilities: [],
  action_count: 1,
  context_contributor_count: 1,
});

afterEach(() => cleanup());

describe('Agent capability list', () => {
  test('shows capability identity, purpose, availability, and resource kinds', () => {
    const item = catalogItem();
    const result = render(
      <I18nextProvider i18n={testI18n}>
        <AgentCapabilityList references={[item.capability]} catalog={[item]} />
      </I18nextProvider>
    );
    const page = within(result.container);

    expect(page.getByText('Search knowledge')).toBeTruthy();
    expect(
      page.getByText('Searches knowledge selected by the current conversation.')
    ).toBeTruthy();
    expect(page.getByText('Available')).toBeTruthy();
    expect(result.container.textContent?.includes('Built-in')).toBe(true);
    expect(result.container.textContent?.includes('nomifun.knowledge@1.0.0')).toBe(true);
    expect(page.getByText('Knowledge base')).toBeTruthy();
    expect(page.getByText('1 actions')).toBeTruthy();
    expect(page.getByText('1 context contributors')).toBeTruthy();
  });

  test('uses an explicit off/startup/requestable control', () => {
    const item = catalogItem();
    const changes: CapabilityPlacement[] = [];
    const result = render(
      <I18nextProvider i18n={testI18n}>
        <AgentCapabilityList
          references={[item.capability]}
          catalog={[item]}
          placementFor={() => 'initial'}
          onPlacementChange={(_capability, placement) => changes.push(placement)}
        />
      </I18nextProvider>
    );
    const page = within(result.container);

    expect(page.getByRole('radio', { name: 'Off' })).toBeTruthy();
    expect(page.getByRole('radio', { name: 'At startup' })).toBeTruthy();
    expect(page.getByRole('radio', { name: 'May request' })).toBeTruthy();
    fireEvent.click(page.getByRole('radio', { name: 'May request' }));

    expect(changes).toEqual(['on_demand']);
  });

  test('renders every catalog capability instead of only a selected count', () => {
    const first = catalogItem();
    const second = catalogItem('workspace.read', 'Read workspace');
    const result = render(
      <I18nextProvider i18n={testI18n}>
        <AgentCapabilityList
          references={[first.capability, second.capability]}
          catalog={[first, second]}
        />
      </I18nextProvider>
    );

    expect(result.container.textContent?.includes('Search knowledge')).toBe(true);
    expect(result.container.textContent?.includes('Read workspace')).toBe(true);
    expect(result.container.textContent?.includes('knowledge.search@1.0.0')).toBe(true);
    expect(result.container.textContent?.includes('workspace.read@1.0.0')).toBe(true);
  });

  test('keeps a missing catalog capability visible and unavailable', () => {
    const reference = {
      id: asCapabilityId('plugin.missing'),
      version: '2.0.0',
    };
    const result = render(
      <I18nextProvider i18n={testI18n}>
        <AgentCapabilityList references={[reference]} catalog={[]} />
      </I18nextProvider>
    );
    const page = within(result.container);

    expect(page.getByText('plugin.missing')).toBeTruthy();
    expect(page.getAllByText('Unavailable')).toHaveLength(2);
    expect(result.container.textContent?.includes('plugin.missing@2.0.0')).toBe(true);
    expect(page.getByText('No implementation is available.')).toBeTruthy();
    expect(page.queryByText('No additional resources')).toBeNull();
  });
});
