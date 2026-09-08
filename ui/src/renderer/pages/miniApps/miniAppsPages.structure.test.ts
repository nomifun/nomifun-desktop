/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { existsSync, readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const listSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const runnerSource = readFileSync(
  new URL('./RunnerPage.tsx', import.meta.url),
  'utf8'
);
const dialogSource = readFileSync(
  new URL('./MiniAppCreateProjectDialog.tsx', import.meta.url),
  'utf8'
);
const modelSource = readFileSync(new URL('./model.ts', import.meta.url), 'utf8');
const typesSource = readFileSync(
  new URL('../../../common/types/miniAppPlatform.ts', import.meta.url),
  'utf8'
);

describe('MiniApp M1 product surfaces', () => {
  test('Library uses only the clean-start M1 collection and project routes', () => {
    expect(listSource.includes('ipcBridge.miniapps.library.invoke()')).toBe(true);
    expect(listSource.includes('<MiniAppCreateProjectDialog')).toBe(true);
    expect(
      dialogSource.includes('ipcBridge.miniapps.createProject.invoke({')
    ).toBe(true);
    expect(dialogSource.includes('expected_library_revision: libraryRevision')).toBe(
      true
    );
    expect(dialogSource.includes("kind: 'ui_only'")).toBe(true);
    expect(dialogSource.includes("<Radio value='service'>")).toBe(false);
    expect(listSource.includes('navigate(`/mini-apps/${app.miniapp_id}`)')).toBe(
      true
    );
  });

  test('Workshop reads a branded owner-scoped M1 detail', () => {
    expect(runnerSource.includes('parseMiniAppId(rawId)')).toBe(true);
    expect(
      runnerSource.includes('ipcBridge.miniapps.getWorkshop.invoke({')
    ).toBe(true);
    expect(runnerSource.includes('miniapp_id: miniappId')).toBe(true);
    expect(
      runnerSource.includes(
        "isBackendHttpError(error) && error.status === 404"
      )
    ).toBe(true);
    expect(runnerSource.includes("navigate('/mini-apps')")).toBe(true);
  });

  test('Workshop exposes the delivered Source, Build, and Ready workflow', () => {
    for (const step of ['source', 'build', 'ready']) {
      expect(
        runnerSource.includes(`miniApps.workshop.workflow.${step}`)
      ).toBe(true);
    }
    for (const fact of [
      'source_snapshot_digest',
      'dependency_lock_digest',
      'project_revision',
      'build_generation',
      'active_operation',
    ]) {
      expect(runnerSource.includes(fact)).toBe(true);
    }
    expect(runnerSource.includes('ipcBridge.miniapps.publish')).toBe(false);
    expect(runnerSource.includes('ipcBridge.miniapps.build.invoke(')).toBe(true);
    expect(
      runnerSource.includes('ipcBridge.miniapps.cancelBuild.invoke({')
    ).toBe(true);
    expect(runnerSource.includes('ipcBridge.miniapps.delete')).toBe(false);
  });

  test('legacy single-HTML runtime and conversation authoring are absent', () => {
    for (const source of [listSource, runnerSource, dialogSource, modelSource]) {
      expect(source.includes('miniapp.html')).toBe(false);
      expect(source.includes('/serve')).toBe(false);
      expect(source.includes('srcDoc')).toBe(false);
      expect(source.includes('<iframe')).toBe(false);
      expect(source.includes('provisionWorkspace')).toBe(false);
      expect(source.includes('source_conversation_id')).toBe(false);
      expect(source.includes('useMiniAppIterate')).toBe(false);
      expect(source.includes('MiniAppFrame')).toBe(false);
      expect(source.includes('MiniAppImportDialog')).toBe(false);
    }
  });

  test('retired UI modules are physically deleted', () => {
    for (const path of [
      './contract.ts',
      './MiniAppFrame.tsx',
      './MiniAppImportDialog.tsx',
      './importConversion.ts',
      './importReport.ts',
      './relativeTime.ts',
      './useMiniAppIterate.ts',
      './useMiniAppMutations.tsx',
    ]) {
      expect(existsSync(new URL(path, import.meta.url))).toBe(false);
    }
  });

  test('M1 wire types expose Product/Project/Release state and no legacy record', () => {
    for (const typeName of [
      'MiniAppSummary',
      'MiniAppWorkshop',
      'MiniAppReadyRelease',
      'MiniAppCapabilityContribution',
      'CreateMiniAppProjectRequest',
    ]) {
      expect(typesSource.includes(`interface ${typeName}`)).toBe(true);
    }
    for (const legacyField of [
      'html_size',
      'published_at',
      'has_unpublished_changes',
      'source_conversation_id',
      'source_path',
    ]) {
      expect(typesSource.includes(legacyField)).toBe(false);
    }
  });
});
