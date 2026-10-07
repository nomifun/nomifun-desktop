import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import ResourcePageBoundary from './ResourcePageBoundary';
import { CANVASES_PATH, MATERIALS_PATH, PROMPTS_PATH, TEMPLATES_PATH } from './resourceRoutes';

describe('resource page boundary', () => {
  test('hosts each retained page and the same themed portal without product chrome', () => {
    for (const path of [CANVASES_PATH, MATERIALS_PATH, PROMPTS_PATH, TEMPLATES_PATH]) {
      const html = renderToStaticMarkup(<MemoryRouter initialEntries={[path]}><Routes><Route element={<ResourcePageBoundary />}><Route path={path} element={<div data-page-content>content</div>} /></Route></Routes></MemoryRouter>);
      expect(html).toContain('data-page-content');
      expect(html).toContain('id="resource-page-portal-root"');
      expect(html).not.toContain('data-creative-studio-focus-shell');
    }
    const css = readFileSync(new URL('./ResourcePageBoundary.module.css', import.meta.url), 'utf8');
    expect(css).toContain('color-scheme: inherit');
    expect(css).toContain(":global([data-theme='dark']) .shell");
  });
});
