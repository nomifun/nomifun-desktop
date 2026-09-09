/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(
  new URL('./MiniAppTransferDialog.tsx', import.meta.url),
  'utf8'
);

describe('MiniApp Transfer dialog source contract', () => {
  test('exposes the fixed parent-page integration surface', () => {
    for (const contract of [
      "'export_backup'",
      "'import_backup'",
      'visible: boolean;',
      'libraryRevision: number;',
      'workshop?: MiniAppWorkshop | null;',
      'onCancel: () => void;',
      'onImported: (workshop: MiniAppWorkshop) => void;',
      'operation: MiniAppOperationSummary,',
      'destinationPath: string',
    ]) {
      expect(source.includes(contract)).toBe(true);
    }
  });

  test('selects an existing directory and keeps import source_path at that selection', () => {
    expect(
      source.includes("ipcBridge.dialog.showOpen.invoke({")
    ).toBe(true);
    expect(source.includes("properties: ['openDirectory']")).toBe(true);
    expect(source.includes("setSourcePath(selectedPath)")).toBe(true);
    expect(source.includes('source_path: sourcePath')).toBe(true);
  });

  test('reads only summary metadata from the mode-specific manifest', () => {
    expect(source.includes("? 'bundle.json'")).toBe(true);
    expect(source.includes("'release/artifact.json'")).toBe(true);
    expect(
      source.includes('ipcBridge.fs.readFile.invoke({')
    ).toBe(true);
    expect(source.includes("stringField(display, 'name')")).toBe(true);
    expect(
      source.includes("stringField(bundle, 'bundle_digest')")
    ).toBe(true);
    expect(
      source.includes("stringField(artifact, 'artifact_digest')")
    ).toBe(true);
    expect(source.includes('backendAuthority')).toBe(true);
  });

  test('builds a new export target and sends exact release fences', () => {
    expect(
      source.includes(
        'const targetPath = joinLocalPath(parentPath, folderName.trim());'
      )
    ).toBe(true);
    expect(source.includes('ipcBridge.miniapps.share.invoke(')).toBe(true);
    expect(source.includes('miniAppShareRequest(')).toBe(true);
    expect(source.includes('onExported(operation, targetPath)')).toBe(true);
  });

  test('routes Share Bundle and Artifact imports through their dedicated bridges', () => {
    expect(
      source.includes('ipcBridge.miniapps.importShare.invoke({')
    ).toBe(true);
    expect(
      source.includes('expected_bundle_digest: importSummary.bundleDigest!')
    ).toBe(true);
    expect(
      source.includes('expected_release_digest: importSummary.artifactDigest')
    ).toBe(true);
    expect(
      source.includes('ipcBridge.miniapps.importArtifact.invoke({')
    ).toBe(true);
    expect(
      source.includes('expected_artifact_digest: importSummary.artifactDigest')
    ).toBe(true);
    expect(
      source.includes("source_path: joinLocalPath(sourcePath, 'release')")
    ).toBe(true);
    expect(source.includes('onImported(imported)')).toBe(true);
    expect(source.includes('ipcBridge.miniapps.importBackup.invoke({')).toBe(
      true
    );
    expect(source.includes('expected_backup_metadata_digest:')).toBe(true);
  });
});
