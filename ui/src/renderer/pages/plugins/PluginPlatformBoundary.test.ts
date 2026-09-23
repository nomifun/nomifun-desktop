import { readFileSync } from 'node:fs';
import { expect, test } from 'bun:test';

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8');

test('Library, Creator and Detail use only the unified bridge', () => {
  const source = [
    read('./PluginLibraryPage.tsx'),
    read('./PluginCreatorPage.tsx'),
    read('./PluginRunPage.tsx'),
    read('./PluginImportDialog.tsx'),
  ].join('\n');
  expect(source).toContain('pluginPlatform.drafts');
  expect(source).toContain('pluginPlatform.plugins');
  expect(source).not.toMatch(/pluginRuntimes|pluginRuntimeProduct|ipcBridge\.plugins/);
  expect(source).not.toMatch(/Candidate|Publish|AutoApply|AutoPublish|Mount|Project/);
});

test('Preview and installed UI share one Surface host with no in-frame fake storage', () => {
  const creator = read('./PluginCreatorPage.tsx');
  const detail = read('./PluginRunPage.tsx');
  const surface = read('./PluginSurfacePanel.tsx');
  expect(creator).toContain('<PluginSurfacePanel');
  expect(detail).toContain('<PluginSurfacePanel');
  expect(surface).toContain('pluginPlatform.surface.bridge.invoke');
  expect(`${creator}\n${surface}`).not.toContain('srcDoc');
  expect(`${creator}\n${surface}`).not.toContain('new Map');
  expect(surface).toContain("sandbox='allow-scripts'");
});

test('Draft generation exposes the persisted generating revision and real cancel command', () => {
  const creator = read('./PluginCreatorPage.tsx');
  const bridge = read('../../../common/adapter/pluginPlatformBridge.ts');
  expect(creator).toContain("status: 'generating'");
  expect(creator).toContain('revision: current.summary.revision + 1');
  expect(creator).toContain('pluginPlatform.drafts.cancelGeneration.invoke');
  expect(bridge).toContain("`${draftPath(draft_id)}/cancel`");
});

test('every successful Chat or file edit reloads the same temporary Preview adapter', () => {
  const creator = read('./PluginCreatorPage.tsx');
  expect(creator).toContain('const reloadPreview = useCallback');
  expect(creator.match(/await reloadPreview\(next\)/g)?.length).toBeGreaterThanOrEqual(3);
  expect(creator).toContain('ownedGenerationDraft.current === draft.summary.draft_id');
  expect(creator).toContain('permissions: previewPermissions.filter');
  expect(creator).toContain('targetManifest.secret_slots.flatMap');
  expect(creator).toContain('config,');
});

test('remote WebUI keeps Plugin reads while every local mutation is desktop-gated', () => {
  const library = read('./PluginLibraryPage.tsx');
  const creator = read('./PluginCreatorPage.tsx');
  const detail = read('./PluginRunPage.tsx');
  const importing = read('./PluginImportDialog.tsx');
  const configuration = read('./PluginConfigurationDialog.tsx');
  const organization = read('./pluginLibraryState.ts');
  const surface = read('./PluginSurfacePanel.tsx');
  const agentTemplate = read('../agentSettings/AgentContributionOrder.tsx');
  for (const source of [library, creator, detail, importing, configuration, organization, surface, agentTemplate]) {
    expect(source).toContain('isDesktopShell');
  }
  expect(library).toContain("t('pluginPlatform.readOnly.body')");
  expect(creator).toContain('if (!desktopShell) return');
  expect(detail).toContain('desktopShell &&');
  expect(importing).toContain('if (!desktopShell) return');
  expect(configuration).toContain('if (!desktopShell || !detail) return');
  expect(organization).toContain('if (!isDesktopShell())');
  expect(surface).toContain('const source = desktopShell ?');
  expect(agentTemplate).toContain('if (!desktopShell || disabled || pending.current) return');
});

test('Preview and Config bind only listed Host Credential references', () => {
  const creator = read('./PluginCreatorPage.tsx');
  const configuration = read('./PluginConfigurationDialog.tsx');
  const importing = read('./PluginImportDialog.tsx');
  for (const source of [creator, configuration, importing]) {
    expect(source).toContain('pluginPlatform.credentials.list.invoke');
    expect(source).toContain('<Select');
    expect(source).toContain('reference.enabled');
    expect(source).toContain('disabled: !reference.enabled');
  }
  expect(creator).not.toMatch(/provider:|connection:/);
  expect(configuration).not.toMatch(/provider:|connection:/);
  expect(configuration).toContain('credentialUnavailableSelected');
  expect(creator).toContain('preview.credentialUnavailable');
  expect(creator).toContain('credential_bindings: credentialBindings');
  expect(importing).toContain('credential_bindings: credentialBindings');
  expect(creator).toContain('value={draftConfig}');
  expect(importing).toContain('value={config}');
});

test('UI-only, headless and mixed Plugins stay one product flow', () => {
  const library = read('./PluginLibraryPage.tsx');
  const detail = read('./PluginRunPage.tsx');
  const model = read('./pluginPlatformModel.ts');
  expect(library).toContain('pluginShape(plugin)');
  expect(detail).toContain('summary.has_ui');
  expect(model).toContain('value.has_ui && value.has_service');
  expect(detail).toContain('manifest.actions');
  expect(detail).toContain('manifest.bindings');
  expect(detail).not.toContain('setServiceRunning');
});

test('Import and restore explain trust, Backup, permissions and data loss', () => {
  const importing = read('./PluginImportDialog.tsx');
  const detail = read('./PluginRunPage.tsx');
  expect(importing).toContain('trusted_local_service');
  expect(importing).toContain('credential_slots_to_rebind');
  expect(importing).toContain('confirmation_required');
  expect(importing).not.toMatch(/expected_(?:artifact|bundle)_digest/);
  expect(detail).toContain('previous_code_and_data');
  expect(detail).toContain('acknowledge_data_loss');
});
