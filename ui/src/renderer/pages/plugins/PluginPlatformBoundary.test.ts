import { readFileSync } from 'node:fs';
import { expect, test } from 'bun:test';

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8');

test('remote WebUI keeps Plugin reads while every local mutation is desktop-gated', () => {
  const library = read('./PluginLibraryPage.tsx');
  const creator = read('./PluginAuthoringArtifacts.tsx');
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
  expect(creator).toContain('!isDesktopShell()');
  expect(detail).toContain('desktopShell &&');
  expect(importing).toMatch(/if \(!desktopShell(?: \|\| [^)]+)?\) return/);
  expect(configuration).toContain('if (!desktopShell || !detail) return');
  expect(organization).toContain('if (!isDesktopShell())');
  expect(surface).toContain('const source = desktopShell ?');
  expect(agentTemplate).toContain('if (!desktopShell || disabled || pending.current) return');
});

test('Preview and Config bind only listed Host Credential references', () => {
  const creator = read('./PluginAuthoringArtifacts.tsx');
  const configuration = read('./PluginConfigurationDialog.tsx');
  const importing = read('./PluginImportDialog.tsx');
  for (const source of [configuration, importing]) {
    expect(source).toContain('pluginPlatform.credentials.list.invoke');
    expect(source).toContain('<Select');
    expect(source).toContain('reference.enabled');
    expect(source).toContain('disabled: !reference.enabled');
  }
  expect(creator).not.toMatch(/provider:|connection:/);
  expect(configuration).not.toMatch(/provider:|connection:/);
  expect(configuration).toContain('credentialUnavailableSelected');
  expect(importing).toContain('credential_bindings: credentialBindings');
  expect(importing).toContain('value={config}');
});
