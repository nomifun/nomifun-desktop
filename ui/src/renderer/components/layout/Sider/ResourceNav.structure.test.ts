import { describe, expect, test } from 'bun:test';
import { existsSync, readFileSync } from 'node:fs';
const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const router = readFileSync(new URL('../Router.tsx', import.meta.url), 'utf8');
const titlebar = readFileSync(new URL('../Titlebar/index.tsx', import.meta.url), 'utf8');

describe('desktop resource navigation', () => {
  test('keeps global navigation and places canvases after companions, prompts/templates after assets', () => {
    const companion = source.indexOf('<SiderNomiEntry');
    const canvas = source.indexOf("label={t('creativeStudio.navigation.canvases'");
    const assets = source.indexOf('<SiderAssetLibraryEntry');
    const prompts = source.indexOf("label={t('creativeStudio.navigation.prompts'");
    const templates = source.indexOf("label={t('creativeStudio.navigation.templates'");
    expect(companion).toBeGreaterThan(-1);
    expect(canvas).toBeGreaterThan(companion);
    expect(prompts).toBeGreaterThan(assets);
    expect(templates).toBeGreaterThan(prompts);
    expect(source).not.toContain('CreativeStudioSider');
    expect(source).not.toContain('SiderCreativeStudioEntry');
  });
  test('mounts retained resource pages and does not mount old product or generation pages', () => {
    for (const route of ['CANVASES_PATH', 'CANVAS_PATTERN', 'MATERIALS_PATH', 'PROMPTS_PATH', 'TEMPLATES_PATH']) {
      expect(router).toContain(`path={${route}}`);
    }
    expect(router).not.toContain("path='/workshop");
    expect(router).not.toContain('workbenches/product');
    expect(existsSync(new URL('../../../pages/creativeStudio/workbenches', import.meta.url))).toBe(false);
  });
  test('keeps canvas save gating ahead of every primary rail and titlebar navigation', () => {
    expect(source.indexOf('await requestCreativeStudioBeforeLeave()')).toBeLessThan(source.indexOf('await navigate(pending.target'));
    expect(source).toContain('navTo(isSettings ?');
    expect(titlebar).toContain('await requestCreativeStudioBeforeLeave()');
    expect(titlebar).not.toContain('workbenchSiderChannels');
    expect(titlebar).toContain('isSessionRoute || isAgentRoute');
  });
});
