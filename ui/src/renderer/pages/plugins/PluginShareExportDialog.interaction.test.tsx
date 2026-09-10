/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';

import { ipcBridge } from '@/common';
import {
  parsePluginArtifactId,
  parsePluginCandidateId,
  parsePluginMountId,
  parsePluginProjectId,
} from '@/common/types/ids';
import type { PluginProjectDetail, SharePluginRequest } from '@/common/types/pluginPlatform';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import en from '@/renderer/services/i18n/locales/en-US/pluginWorkbench.json';
import PluginShareExportDialog from './PluginShareExportDialog';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: { pluginWorkbench: en } } },
  interpolation: { escapeValue: false },
});

const projectId = parsePluginProjectId('0190f5fe-7c00-7a00-8000-000000000071');
const candidateId = parsePluginCandidateId('0190f5fe-7c00-7a00-8000-000000000072');
const artifactId = parsePluginArtifactId('0190f5fe-7c00-7a00-8000-000000000073');
const mountId = parsePluginMountId('0190f5fe-7c00-7a00-8000-000000000074');

const detail: PluginProjectDetail = {
  summary: {
    project_id: projectId,
    project_revision: 8,
    display_name: 'Share Plugin',
    linked_mount_id: mountId,
    source_state: 'editable',
    build_generation: 3,
    ready_candidate: {
      candidate_id: candidateId,
      candidate_digest: 'd'.repeat(64),
    },
    apply_mode: 'ask_before_apply',
    auto_apply_authorization_revision: 0,
    updated_at_ms: 1,
  },
  source_snapshot_digest: 'a'.repeat(64),
  dependency_lock_digest: 'b'.repeat(64),
  direct_dependencies: {},
  ready: {
    candidate: {
      candidate_id: candidateId,
      candidate_digest: 'd'.repeat(64),
    },
    origin: 'build',
    target: {
      package_id: 'example.share',
      package_version: '1.0.0',
      artifact_id: artifactId,
      artifact_digest: 'c'.repeat(64),
      manifest_digest: 'e'.repeat(64),
    },
    project_build_generation: 3,
    test: {
      status: 'passed',
      candidate_id: candidateId,
      candidate_digest: 'd'.repeat(64),
    },
    impact: {
      compatibility: 'compatible',
      changed_contracts: [],
      affected_consumers: [],
      can_apply: true,
      can_auto_apply: false,
      blocking_reasons: [],
    },
  },
};

const originalShowOpen = ipcBridge.dialog.showOpen.invoke;

afterEach(() => {
  cleanup();
  ipcBridge.dialog.showOpen.invoke = originalShowOpen;
});

describe('Plugin Share export dialog', () => {
  test('submits exact Ready Candidate CAS and no user-data fields', async () => {
    ipcBridge.dialog.showOpen.invoke = async () => ['C:\\exports'];
    let request: SharePluginRequest | undefined;
    render(
      <I18nextProvider i18n={testI18n}>
        <PluginShareExportDialog
          visible
          detail={detail}
          loading={false}
          onCancel={() => {}}
          onSubmit={(value) => {
            request = value;
          }}
        />
      </I18nextProvider>
    );
    const dialog = within(document.body);
    await dialog.findByRole('dialog', { name: 'Export Plugin Share Bundle' });
    fireEvent.click(dialog.getByRole('button', { name: 'Choose folder' }));
    await waitFor(() => expect(dialog.getByDisplayValue('C:\\exports')).toBeDefined());
    fireEvent.click(dialog.getByRole('button', { name: 'Export Share Bundle' }));
    await waitFor(() => expect(request).toBeDefined());

    expect(request).toEqual({
      project_id: projectId,
      expected_project_revision: 8,
      source: 'ready_candidate',
      candidate_id: candidateId,
      expected_candidate_digest: 'd'.repeat(64),
      destination_path: 'C:\\exports\\Share Plugin-share',
      include_source: true,
    });
    expect(JSON.stringify(request)).not.toContain('credential');
    expect(JSON.stringify(request)).not.toContain('data_dir');
  });
});
