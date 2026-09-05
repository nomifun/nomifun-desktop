import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const sourceFile = (name: string) =>
  readFileSync(new URL(`./${name}`, import.meta.url), 'utf8');

const between = (source: string, startMarker: string, endMarker: string): string => {
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start + startMarker.length);
  expect(start).toBeGreaterThanOrEqual(0);
  expect(end).toBeGreaterThan(start);
  return source.slice(start, end);
};

describe('Agent Settings host-managed process session wiring', () => {
  test('completes host bindings before Preview, Save, and Test', () => {
    const controller = sourceFile('useAgentSettingsController.ts');

    expect(
      between(controller, 'const runPreview =', 'const saveRevision =').includes(
        'resolveHostManagedResourceBindings('
      )
    ).toBe(true);
    expect(
      between(controller, 'const saveRevision =', 'const runTest =').includes(
        'resolveHostManagedResourceBindings('
      )
    ).toBe(true);
    expect(
      between(controller, 'const runTest =', 'const dirty =').includes(
        'resolveHostManagedResourceBindings('
      )
    ).toBe(true);
    expect(
      controller.includes('setSavedDraft(response.revision ? savedResponseDraft : null)')
    ).toBe(true);
  });

  test('synchronizes capability selection and workspace edits immediately', () => {
    const editor = sourceFile('AgentPresetEditor.tsx');

    expect(
      between(editor, 'const nextDraftForCapability =', 'const place =').includes(
        'resolveHostManagedResourceBindings('
      )
    ).toBe(true);
    expect(
      between(editor, 'const updateWorkspaceSelection =', 'const selectedSkills =').includes(
        'resolveHostManagedResourceBindings('
      )
    ).toBe(true);
  });

  test('keeps the template process session derived and non-editable', () => {
    const overview = sourceFile('OfficialTemplateOverview.tsx');

    expect(
      /resource\.resource_kind === 'process_session'\s*\?\s*selectedWorkspaceRoot/.test(
        overview
      )
    ).toBe(true);
    expect(
      overview.includes(
        "!['workspace', 'process_session', 'knowledge_base', 'mcp_server'].includes("
      )
    ).toBe(true);
    expect(
      between(
        overview,
        "{resource.resource_kind === 'process_session' && (",
        "{!['workspace', 'process_session'"
      ).includes('<Input')
    ).toBe(false);
  });
});
