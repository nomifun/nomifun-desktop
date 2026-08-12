import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const readSource = (relativePath: string) =>
  readFileSync(new URL(relativePath, import.meta.url), 'utf8');

describe('default image-generation model', () => {
  test('uses the canonical model-owned preference and has no legacy tool key', () => {
    const keys = readSource('../../../common/config/configKeys.ts');

    expect(keys.includes("'models.default.imageGeneration'")).toBe(true);
    expect(keys.includes("'tools.imageGenerationModel'")).toBe(false);
  });

  test('the image section wires the exact image-generation task selector', () => {
    const imageSection = readSource('./ImageModelsContent.tsx');
    const panel = readSource('./ModalityModelsPanel.tsx');

    expect(
      imageSection.includes(
        "defaultModelPreferenceKey='models.default.imageGeneration'",
      ),
    ).toBe(true);
    expect(panel.includes("task='image_generation'")).toBe(true);
    expect(panel.includes("useModelsForTask('image_generation')")).toBe(true);
    expect(panel.includes('disabled={noCandidates || isSavingDefault}')).toBe(
      true,
    );
    expect(
      panel.includes("t('settings.modelHub.creation.defaultNoModels')"),
    ).toBe(true);
  });

  test('saving and clearing go through the canonical config service key', () => {
    const panel = readSource('./ModalityModelsPanel.tsx');

    expect(panel.includes('configService.set(preferenceKey, next)')).toBe(true);
    expect(panel.includes('configService.remove(preferenceKey)')).toBe(true);
    expect(panel.includes('configService.reload()')).toBe(true);
    expect(panel.includes('SerializedLatestWriteQueue')).toBe(true);
  });
});
