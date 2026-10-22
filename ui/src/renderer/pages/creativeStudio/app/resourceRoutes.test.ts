import { describe, expect, test } from 'bun:test';
import { ASSET_LIBRARY_PATH, CANVASES_PATH, MATERIALS_PATH, PROMPTS_PATH, TEMPLATES_PATH, canvasPath, matchCanvasPath, resourceSectionForPath, resourceSpaceSectionForPath } from './resourceRoutes';
import { normalizeCanvasResumeLocation, readCanvasResumeLocation, rememberCanvasResumeLocation } from './canvasResumeLocation';

describe('resource destinations', () => {
  test('keeps global canvases separate from companion identity and materials separate from templates', () => {
    expect(canvasPath('a/b')).toBe('/nomi/canvases/a%2Fb');
    expect(matchCanvasPath('/nomi/canvases/a%2Fb?view=canvas')).toEqual({ canvasId: 'a/b' });
    expect(resourceSectionForPath(CANVASES_PATH)).toBe('canvases');
    expect(resourceSectionForPath(MATERIALS_PATH)).toBe('assets');
    expect(resourceSectionForPath(PROMPTS_PATH)).toBe('prompts');
    expect(resourceSectionForPath(TEMPLATES_PATH)).toBe('templates');
    for (const path of ['/nomi', '/nomi/canvases-other', '/workshop/image', '/workshop/video', '/workshop/canvas/id']) {
      expect(resourceSectionForPath(path)).toBeNull();
    }
  });
  test('matches only materials and templates as resource space tabs', () => {
    expect(resourceSpaceSectionForPath(ASSET_LIBRARY_PATH)).toBe('assets');
    expect(resourceSpaceSectionForPath(`${MATERIALS_PATH}?q=reference`)).toBe('assets');
    expect(resourceSpaceSectionForPath(TEMPLATES_PATH)).toBe('templates');
    expect(resourceSpaceSectionForPath(PROMPTS_PATH)).toBeNull();
    expect(resourceSpaceSectionForPath(CANVASES_PATH)).toBeNull();
  });
  test('validates canvas resume paths and rejects non-canvas/external destinations', () => {
    expect(normalizeCanvasResumeLocation('/nomi/canvases/a?tab=agent#node')).toBe('/nomi/canvases/a?tab=agent#node');
    for (const value of [null, '//example.com/nomi/canvases', '/\\example.com/nomi/canvases', '/asset-library/materials', '/nomi']) {
      expect(normalizeCanvasResumeLocation(value)).toBeNull();
    }
  });
  test('migrates the old canvas location once after successfully saving its destination', () => {
    const values = new Map([['nomifun:creative-studio:canvases-resume-location', '/workshop/canvas/id?tab=agent#node']]);
    const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); }, removeItem: (key: string) => { values.delete(key); } };
    expect(readCanvasResumeLocation(storage)).toBe('/nomi/canvases/id?tab=agent#node');
    expect(values.has('nomifun:creative-studio:canvases-resume-location')).toBe(false);
    expect(readCanvasResumeLocation(storage)).toBe('/nomi/canvases/id?tab=agent#node');
    expect(rememberCanvasResumeLocation('/asset-library/prompts', storage)).toBeNull();
    expect(readCanvasResumeLocation(storage)).toBe('/nomi/canvases/id?tab=agent#node');
  });
});
