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
    expect(panel.includes("'models.default.imageGeneration': { task: 'image_generation', traits: [], technical: [] }")).toBe(true);
    expect(panel.includes('task={spec.task}')).toBe(true);
    expect(panel.includes('[...spec.traits]')).toBe(true);
    expect(panel.includes('[...spec.technical]')).toBe(true);
    expect(panel.includes('disabled={noCandidates || isSavingDefault}')).toBe(
      true,
    );
    expect(panel.includes("'settings.modelHub.creation.defaultNoModels'")).toBe(true);
  });

  test('the vision section stores an exact vision-and-tools auxiliary route', () => {
    const keys = readSource('../../../common/config/configKeys.ts');
    const visionSection = readSource('./VisionModelsContent.tsx');
    const panel = readSource('./ModalityModelsPanel.tsx');

    expect(keys.includes("'models.default.vision'")).toBe(true);
    expect(
      visionSection.includes("defaultModelPreferenceKey='models.default.vision'"),
    ).toBe(true);
    expect(
      panel.includes("task: 'chat'")
        && panel.includes("traits: ['vision_input']")
        && panel.includes("technical: ['function_calling']"),
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
