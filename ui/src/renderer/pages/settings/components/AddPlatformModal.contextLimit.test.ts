import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (relative: string) => readFileSync(new URL(relative, import.meta.url), 'utf8');

describe('task capability context window control', () => {
  test('keeps common presets in the shared task card', () => {
    const editorSource = readSource('./ModelDefinitionEditor.tsx');
    const selectSource = readSource('./ContextLimitSelect.tsx');

    expect(selectSource.includes('CONTEXT_WINDOW_OPTIONS')).toBe(true);
    expect(selectSource.includes('value: 32_000')).toBe(true);
    expect(selectSource.includes('value: 64_000')).toBe(true);
    expect(selectSource.includes('value: 128_000')).toBe(true);
    expect(selectSource.includes('value: 200_000')).toBe(true);
    expect(selectSource.includes('value: 1_000_000')).toBe(true);
    expect(editorSource.includes('<ContextLimitSelect')).toBe(true);
    expect(editorSource.includes('value={capability.contextLimit}')).toBe(true);
    expect(editorSource.includes('{ contextLimit }')).toBe(true);
  });

  test('all create and edit surfaces reuse the same capability editor', () => {
    for (const relative of ['./AddPlatformModal.tsx', './AddModelModal.tsx', './ModelAdvancedEditor.tsx']) {
      const source = readSource(relative);
      expect(source.includes('<ModelDefinitionEditor')).toBe(true);
      expect(source.includes("callConfigFooterPlacement='modal'")).toBe(true);
      expect(source.includes('onCallConfigFocusChange=')).toBe(true);
    }
  });

  test('keeps call-configuration tabs goal driven and task scoped', () => {
    const editorSource = readSource('./ModelDefinitionEditor.tsx');

    for (const intent of ['overview', 'connection', 'limits', 'protocol', 'diagnostics']) {
      expect(editorSource.includes(`data-call-config-intent='${intent}'`) || editorSource.includes(`key: '${intent}'`)).toBe(
        true
      );
    }
    expect(editorSource.includes('cancelCallConfig')).toBe(true);
    expect(editorSource.includes('applyCallConfig')).toBe(true);
    expect(editorSource.includes('data-call-config-context')).toBe(true);
  });

  test('serializes context as part of each full capability save', () => {
    const advancedSource = readSource('./providerModelAdvanced.ts');
    const addProviderSource = readSource('./AddPlatformModal.tsx');
    const addModelSource = readSource('./AddModelModal.tsx');

    expect(advancedSource.includes('context_limit: capability.contextLimit')).toBe(true);
    expect(addProviderSource.includes('initial_model')).toBe(true);
    expect(addProviderSource.includes('capabilityInputsFromDefinition')).toBe(true);
    expect(addModelSource.includes('ipcBridge.providerModel.save.invoke')).toBe(true);
  });
});
