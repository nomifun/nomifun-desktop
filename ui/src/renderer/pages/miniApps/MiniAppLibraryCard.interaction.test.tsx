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
import type { MiniAppSummary } from '@/common/types/miniAppPlatform';
import { parseMiniAppId } from '@/common/types/ids';
import en from '@/renderer/services/i18n/locales/en-US/miniApps.json';
import { MiniAppLibraryCard } from './index';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {
        miniApps: en,
      },
    },
  },
  interpolation: { escapeValue: false },
});

const app: MiniAppSummary = {
  miniapp_id: parseMiniAppId('0190f5fe-7c00-7a00-8000-0000000000b1'),
  product_revision: 4,
  display_name: 'Status Board',
  description: 'Track the current project status.',
  kind: 'ui_only',
  lifecycle: 'enabled',
  releases: {
    pointer_revision: 7,
    active_release_epoch: 3,
    active: {
      release_id: 'active-release',
      artifact_id: 'active-artifact',
      release_digest: 'a'.repeat(64),
      manifest_digest: 'b'.repeat(64),
    },
  },
  service_health: { state: 'not_applicable' },
  surface_available: true,
  updated_at_ms: 1,
};

afterEach(() => cleanup());

describe('MiniApp Library card accessibility', () => {
  test('names the card and disambiguates each Open workbench action', () => {
    let opened: MiniAppSummary | null = null;
    const view = render(
      <I18nextProvider i18n={i18n}>
        <MiniAppLibraryCard
          app={app}
          locale='en-US'
          onOpen={(next) => {
            opened = next;
          }}
        />
      </I18nextProvider>
    );

    const card = view.getByRole('article', { name: app.display_name });
    expect(card.getAttribute('aria-describedby')).toBe(
      `miniapp-library-card-description-${app.miniapp_id}`
    );

    const openButton = within(card).getByRole('button', {
      name: `Open workbench: ${app.display_name}`,
    });
    fireEvent.click(openButton);
    expect(opened).toBe(app);
  });
});
