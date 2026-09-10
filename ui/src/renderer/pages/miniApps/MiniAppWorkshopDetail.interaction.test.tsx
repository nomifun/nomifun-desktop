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
import { parseMiniAppId } from '@/common/types/ids';
import type { MiniAppWorkshop } from '@/common/types/miniAppPlatform';
import en from '@/renderer/services/i18n/locales/en-US/miniApps.json';
import { MiniAppWorkshopDetail } from './RunnerPage';

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

const miniappId = parseMiniAppId(
  '0190f5fe-7c00-7a00-8000-0000000000b1'
);

const release = (prefix: string) => ({
  release_id: `${prefix}-release`,
  artifact_id: `${prefix}-artifact`,
  release_digest: prefix.repeat(64).slice(0, 64),
  manifest_digest: `${prefix}manifest`.repeat(16).slice(0, 64),
});

const workshop = (
  lifecycle: 'enabled' | 'disabled' | 'trashed' = 'enabled'
): MiniAppWorkshop => {
  const readyRelease = release('a');
  const activeRelease = release('b');
  const previousRelease = release('c');
  return {
    miniapp: {
      miniapp_id: miniappId,
      product_revision: 4,
      display_name: 'Status Board',
      description: 'Track the current project status.',
      kind: 'ui_only',
      lifecycle,
      releases: {
        pointer_revision: 7,
        active_release_epoch: 3,
        ready: readyRelease,
        active: activeRelease,
        previous: previousRelease,
      },
      service_health: { state: 'not_applicable' },
      surface_available: lifecycle === 'enabled',
      updated_at_ms: 1,
    },
    project_id: 'project-1',
    project_revision: 4,
    publish_mode: 'manual',
    source_state: 'editable',
    build_generation: 2,
    source_snapshot_digest: 'd'.repeat(64),
    dependency_lock_digest: 'e'.repeat(64),
    ready: {
      release: readyRelease,
      project_build_generation: 2,
      created_at_ms: 1,
      kind: 'ui_only',
      test: {
        status: 'not_required',
        release_id: readyRelease.release_id,
        expected_release_digest: readyRelease.release_digest,
      },
      migration_count: 0,
      can_publish: true,
      can_auto_publish: true,
      blocking_reasons: [],
    },
    config_schema: { schema_digest: 'f'.repeat(64), schema: {} },
    config: {
      config_revision: 1,
      schema_digest: 'f'.repeat(64),
      values: {},
      valid: true,
      validation_errors: [],
    },
    credential_bindings_revision: 1,
    credential_slots: [],
    capabilities: [],
  };
};

const renderDetail = (
  value: MiniAppWorkshop,
  callbacks: Partial<{
    onBuild: () => void;
    onShare: () => void;
    onOpenSurface: () => void;
    onRefresh: () => void;
  }> = {}
) => {
  const props: React.ComponentProps<typeof MiniAppWorkshopDetail> = {
    workshop: value,
    locale: 'en-US',
    onBack: () => {},
    onRefresh: callbacks.onRefresh ?? (() => {}),
    onBuild: callbacks.onBuild ?? (() => {}),
    onTest: () => {},
    onCancelBuild: () => {},
    onPublish: () => {},
    onRollback: () => {},
    onSetEnabled: () => {},
    onSetPublishMode: () => {},
    onSetServiceLifecycle: () => {},
    onSetServiceRunning: () => {},
    onRetryService: () => {},
    onTrash: () => {},
    onRestore: () => {},
    onDelete: () => {},
    onRetryDelete: () => {},
    onShare: callbacks.onShare ?? (() => {}),
    onBackup: () => {},
    onOpenSurface: callbacks.onOpenSurface ?? (() => {}),
    onReloadSurface: () => {},
    onCloseSurface: () => {},
    surfaceDescriptor: null,
    busyAction: null,
    refreshing: false,
    building: false,
    canceling: false,
    serviceLifecycle: 'on_demand',
  };
  return render(
    <I18nextProvider i18n={i18n}>
      <MiniAppWorkshopDetail {...props} />
    </I18nextProvider>
  );
};

afterEach(() => cleanup());

describe('MiniApp Workshop desktop actions', () => {
  test('keeps the primary delivery and recovery actions keyboard discoverable', () => {
    let builds = 0;
    let shares = 0;
    let surfaces = 0;
    let refreshes = 0;
    const view = renderDetail(workshop(), {
      onBuild: () => {
        builds += 1;
      },
      onShare: () => {
        shares += 1;
      },
      onOpenSurface: () => {
        surfaces += 1;
      },
      onRefresh: () => {
        refreshes += 1;
      },
    });

    const main = view.getByRole('main', { name: 'Status Board' });
    const toolbar = within(main).getByRole('toolbar', {
      name: 'MiniApp Project: Status Board',
    });
    for (const name of [
      'Build Ready Release: Status Board',
      'Publish: Status Board',
      'Rollback: Status Board',
      'Move to Trash: Status Board',
      'Share: Status Board',
      'Open Surface: Status Board',
      'Refresh: Status Board',
    ]) {
      expect(within(toolbar).getByRole('button', { name })).toBeDefined();
    }

    fireEvent.click(
      within(toolbar).getByRole('button', {
        name: 'Build Ready Release: Status Board',
      })
    );
    fireEvent.click(
      within(toolbar).getByRole('button', { name: 'Share: Status Board' })
    );
    fireEvent.click(
      within(toolbar).getByRole('button', {
        name: 'Open Surface: Status Board',
      })
    );
    fireEvent.click(
      within(toolbar).getByRole('button', { name: 'Refresh: Status Board' })
    );

    expect(builds).toBe(1);
    expect(shares).toBe(1);
    expect(surfaces).toBe(1);
    expect(refreshes).toBe(1);
    expect(
      within(main).getByRole('region', { name: /Release pointers/ })
    ).toBeDefined();
  });

  test('exposes Restore and Permanent Delete only for a trashed MiniApp', () => {
    const view = renderDetail(workshop('trashed'));
    const toolbar = within(view.getByRole('main', { name: 'Status Board' })).getByRole(
      'toolbar'
    );

    expect(
      within(toolbar).getByRole('button', { name: 'Restore: Status Board' })
    ).toBeDefined();
    expect(
      within(toolbar).getByRole('button', {
        name: 'Delete Permanently: Status Board',
      })
    ).toBeDefined();
    expect(
      within(toolbar).queryByRole('button', {
        name: 'Build Ready Release: Status Board',
      })
    ).toBeNull();
  });
});
