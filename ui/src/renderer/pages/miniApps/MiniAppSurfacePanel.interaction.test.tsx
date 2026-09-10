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
import type { MiniAppSurfaceLaunchDescriptor } from '@/common/types/miniAppPlatform';
import { parseMiniAppId } from '@/common/types/ids';
import en from '@/renderer/services/i18n/locales/en-US/miniApps.json';
import MiniAppSurfacePanel from './MiniAppSurfacePanel';

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

const descriptor: MiniAppSurfaceLaunchDescriptor = {
  miniapp_id: parseMiniAppId('0190f5fe-7c00-7a00-8000-0000000000b1'),
  product_revision: 4,
  release_id: 'active-release',
  expected_release_digest: 'a'.repeat(64),
  active_release_epoch: 3,
  surface_session_id: 'surface-session',
  surface_generation: 1,
  surface_capability: 'surface-capability',
  ui_entrypoint: 'ui/index.html',
  kind: 'ui_only',
};

afterEach(() => cleanup());

const renderSurface = (
  overrides: Partial<React.ComponentProps<typeof MiniAppSurfacePanel>> = {}
) => {
  const onReload = () => {
    reloads += 1;
  };
  const onClose = () => {
    closes += 1;
  };
  const view = render(
    <I18nextProvider i18n={i18n}>
      <MiniAppSurfacePanel
        descriptor={descriptor}
        displayName='Status Board'
        reloading={false}
        closing={false}
        onReload={onReload}
        onClose={onClose}
        {...overrides}
      />
    </I18nextProvider>
  );
  return { ...view, onReload, onClose };
};

let reloads = 0;
let closes = 0;

describe('MiniApp Surface accessibility and recovery', () => {
  test('names the Surface controls and exposes a retry after frame failure', () => {
    reloads = 0;
    closes = 0;
    const view = renderSurface({
      descriptor: {
        ...descriptor,
        ui_entrypoint: '/unsafe.html',
      },
    });
    const surface = within(view.container);
    const header = within(surface.getByRole('banner'));

    expect(
      header.getByRole('button', { name: 'Reload Surface: Status Board' })
    ).toBeDefined();
    expect(
      header.getByRole('button', { name: 'Close Surface: Status Board' })
    ).toBeDefined();

    const failure = surface.getByRole('alert');
    expect(failure.textContent?.includes('Surface could not be loaded')).toBe(
      true
    );
    const retry = within(failure).getByRole('button', {
      name: 'Reload Surface: Status Board',
    });
    fireEvent.click(retry);

    expect(reloads).toBe(1);
  });
});
