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
const surfaceSource = readFileSync(
  new URL('./MiniAppSurfacePanel.tsx', import.meta.url),
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
    expect(dialogSource.includes('setKind')).toBe(true);
    expect(dialogSource.includes("value: 'service'")).toBe(true);
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

  test('Workshop exposes the delivered Source, Build, Publish, and Surface workflow', () => {
    for (const step of ['source', 'build', 'ready', 'publish', 'surface']) {
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
    for (const route of [
      'ipcBridge.miniapps.publish.invoke(',
      'ipcBridge.miniapps.rollback.invoke(',
      'ipcBridge.miniapps.setEnabled.invoke(',
      'ipcBridge.miniapps.setPublishMode.invoke(',
      'ipcBridge.miniapps.setServiceRunning.invoke(',
      'ipcBridge.miniapps.retryService.invoke(',
      'ipcBridge.miniapps.openSurface.invoke(',
    ]) {
      expect(runnerSource.includes(route)).toBe(true);
    }
    expect(runnerSource.includes('ipcBridge.miniapps.build.invoke(')).toBe(true);
    expect(runnerSource.includes('serviceLifecycle')).toBe(true);
    expect(runnerSource.includes('publishServiceBody')).toBe(true);
    expect(runnerSource.includes('startService')).toBe(true);
    expect(runnerSource.includes('stopService')).toBe(true);
    expect(runnerSource.includes('retryService')).toBe(true);
    expect(
      runnerSource.includes('ipcBridge.miniapps.cancelBuild.invoke({')
    ).toBe(true);
    expect(runnerSource.includes('ipcBridge.miniapps.delete')).toBe(false);
  });

  test('Surface uses only a capability/epoch/digest-fenced strict iframe', () => {
    const sandboxValue = surfaceSource.match(
      /sandbox=['"]([^'"]+)['"]/
    )?.[1];
    expect(sandboxValue).toBe('allow-scripts allow-forms');
    expect(sandboxValue).not.toContain('allow-same-origin');
    expect(sandboxValue).not.toContain('allow-popups');
    expect(sandboxValue).not.toContain('allow-top-navigation');
    expect(surfaceSource.includes('miniAppSurfaceAssetPath(descriptor)')).toBe(
      true
    );
    expect(surfaceSource.includes("miniApps.surface.frameTitle")).toBe(true);
    expect(surfaceSource.includes('new MessageChannel()')).toBe(true);
    expect(surfaceSource.includes('[channel.port2]')).toBe(true);
    expect(
      surfaceSource.includes('nomifun-miniapp-bridge-challenge-v1')
    ).toBe(true);
    expect(
      surfaceSource.includes('nomifun-miniapp-bridge-handshake-v1')
    ).toBe(true);
    expect(
      surfaceSource.includes("event.origin !== 'null'")
    ).toBe(true);
    expect(surfaceSource.includes('createBridgeNonce()')).toBe(true);
    expect(
      surfaceSource.includes('ipcBridge.miniapps.bridge')
    ).toBe(true);
    expect(surfaceSource.includes("target: 'service'")).toBe(true);
    expect(surfaceSource.includes('target.method')).toBe(true);
    expect(surfaceSource.includes('target.payload')).toBe(true);
    expect(runnerSource.includes('ipcBridge.miniapps.closeSurface')).toBe(true);
    expect(surfaceSource.includes('key={bridgeDescriptorKey}')).toBe(true);
    expect(surfaceSource.includes('key={source}')).toBe(false);
    expect(surfaceSource.includes('onLoad={handleFrameLoad}')).toBe(true);
    expect(
      /if \(bridgeLoadRef\.current\.portTransferred\) \{\s*closeBridge\(\);\s*return;\s*\}/.test(
        surfaceSource
      )
    ).toBe(true);
    const handshakeAcceptAt = surfaceSource.indexOf(
      'data?.type !== BRIDGE_HANDSHAKE_EVENT'
    );
    const transferPortAt = surfaceSource.indexOf('openBridge(frame, nonce);');
    expect(handshakeAcceptAt).toBeGreaterThan(-1);
    expect(transferPortAt).toBeGreaterThan(handshakeAcceptAt);
    expect(surfaceSource.match(/openBridge\(frame, nonce\);/g)?.length).toBe(1);
    expect(
      /const revokeBridge = useCallback\(\(\) => \{\s*bridgeLoadRef\.current\.portTransferred = true;\s*bridgeLoadRef\.current\.handshakeNonce = null;\s*closeBridge\(\);/.test(
        surfaceSource
      )
    ).toBe(true);
    expect(surfaceSource.includes('if (bridgePortRef.current !== hostPort) return;')).toBe(
      true
    );
    expect(surfaceSource.includes('window.__nomiLocalTrust')).toBe(false);
    expect(surfaceSource.includes('window.fetch')).toBe(false);
    const closeHandlerAt = runnerSource.indexOf(
      'const handleCloseSurface = useCallback'
    );
    const closeRequestAt = runnerSource.indexOf(
      'await ipcBridge.miniapps.closeSurface.invoke({',
      closeHandlerAt
    );
    const closeDescriptorAt = runnerSource.indexOf(
      'setSurfaceDescriptor(null);',
      closeRequestAt
    );
    expect(closeHandlerAt).toBeGreaterThan(-1);
    expect(closeRequestAt).toBeGreaterThan(closeHandlerAt);
    expect(closeDescriptorAt).toBeGreaterThan(closeRequestAt);
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
