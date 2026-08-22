/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const pageSource = readFileSync(new URL('./CreativeStudioProjectsPage.tsx', import.meta.url), 'utf8');
const cardSource = readFileSync(new URL('./CreativeStudioProjectCard.tsx', import.meta.url), 'utf8');
const adapterSource = readFileSync(new URL('./projectServiceAdapter.ts', import.meta.url), 'utf8');
const css = readFileSync(new URL('./CreativeStudioProjectsPage.module.css', import.meta.url), 'utf8');

describe('Creative Studio project center product contract', () => {
  test('keeps the compact 1440/1280/1024 card-grid measurements', () => {
    expect(css.includes('max-width: 1152px')).toBe(true);
    expect(css.includes('grid-template-columns: repeat(3, minmax(0, 1fr))')).toBe(true);
    expect(/\.grid\s*\{[\s\S]*?gap:\s*16px;/.test(css)).toBe(true);
    expect(css.includes('min-height: 144px')).toBe(true);
    expect(css.includes('border-radius: 16px')).toBe(true);
    expect(css.includes('@media (max-width: 1279px)')).toBe(true);
    expect(css.includes('grid-template-columns: repeat(2, minmax(0, 1fr))')).toBe(true);
    expect(css.includes('@media (max-width: 639px)')).toBe(true);
    expect(css.includes('grid-template-columns: minmax(0, 1fr)')).toBe(true);
    expect(css.includes('min-height: 360px')).toBe(true);
    expect(css.includes('border-top: 1px solid')).toBe(true);
    expect(css.includes('border-bottom: 1px solid')).toBe(true);
  });

  test('keeps the page heading and card contents compact and center-aligned', () => {
    expect(pageSource.includes('styles.eyebrow')).toBe(false);
    expect(/\.container\s*\{[\s\S]*?padding:\s*16px 24px 28px;[\s\S]*?gap:\s*16px;/.test(css)).toBe(true);
    expect(/\.header\s*\{[\s\S]*?padding-bottom:\s*12px;/.test(css)).toBe(true);
    expect(/\.title\s*\{[\s\S]*?font-size:\s*24px;[\s\S]*?line-height:\s*30px;/.test(css)).toBe(true);
    expect(/\.cardCheckbox\s*\{[\s\S]*?margin-top:\s*3px;/.test(css)).toBe(true);
    expect(/\.cardTitle\s*\{[\s\S]*?font-size:\s*16px;[\s\S]*?line-height:\s*22px;/.test(css)).toBe(true);
    expect(/\.cardStats\s*\{[\s\S]*?margin-top:\s*4px;[\s\S]*?font-size:\s*13px;/.test(css)).toBe(true);
    expect(/\.cardFooter\s*\{[\s\S]*?margin-top:\s*16px;[\s\S]*?padding-top:\s*10px;[\s\S]*?align-items:\s*center;/.test(css)).toBe(true);
    expect(/\.cardTimestamp\s*\{(?![\s\S]*?margin-bottom)/.test(css)).toBe(true);
  });

  test('uses theme-aware elevated card surfaces instead of a hardcoded warm fill', () => {
    expect(css.includes('#f1eee8')).toBe(false);
    expect(css.includes('#ebe6dc')).toBe(false);
    expect(/\.card\s*\{[\s\S]*?border:\s*1px solid[\s\S]*?color-mix\([\s\S]*?box-shadow:/.test(css)).toBe(true);
    expect(/\.card:hover\s*\{[\s\S]*?transform:\s*translateY\(-2px\);[\s\S]*?box-shadow:/.test(css)).toBe(true);
    expect(/\.cardFooter\s*\{[\s\S]*?border-top:\s*1px solid color-mix/.test(css)).toBe(true);
    expect(css.includes(":global([data-theme='dark']) .card")).toBe(true);
  });

  test('keeps creation, archive IO, inline rename, and destructive confirmation explicit', () => {
    expect(pageSource.includes('onOpenProject?.(created)')).toBe(true);
    expect(pageSource.includes("type='file'")).toBe(true);
    expect(pageSource.includes("accept='application/zip,.zip'")).toBe(true);
    expect(pageSource.includes('service.importProjectArchive(file)')).toBe(true);
    expect(pageSource.includes('service.exportProjects(ids)')).toBe(true);
    expect(pageSource.includes('service.renameProject(editingId, title)')).toBe(true);
    expect(pageSource.includes('<Modal')).toBe(true);
    expect(pageSource.includes('service.deleteProjects(ids)')).toBe(true);
    expect(pageSource.includes('creativeStudioProjectsService')).toBe(true);
    expect(adapterSource.includes('creativeProjectRepository')).toBe(true);
    expect(adapterSource.includes('repository.load(id)')).toBe(true);
    expect(adapterSource.includes('connectionCount: project.connectionCount')).toBe(true);
  });

  test('does not add search, sorting, select-all, selection summaries, or a subtitle to the source page', () => {
    expect(pageSource.includes('<Input')).toBe(false);
    expect(pageSource.includes('<Select')).toBe(false);
    expect(pageSource.includes('toggleSelectAll')).toBe(false);
    expect(pageSource.includes('selectedCount')).toBe(false);
    expect(pageSource.includes('clearSelection')).toBe(false);
    expect(pageSource.includes('copy.subtitle')).toBe(false);
    expect(css.includes('.subtitle')).toBe(false);
    expect(css.includes('.selectionBar')).toBe(false);
    expect(css.includes('.controls')).toBe(false);
  });

  test('starts card selection only from its checkbox and uses the established icon library', () => {
    const selectionCalls = cardSource.match(/onToggleSelected\(/g) ?? [];
    expect(selectionCalls.length).toBe(1);
    expect(cardSource.includes("onClick={(event) => event.stopPropagation()}")).toBe(true);
    expect(cardSource.includes("from '@icon-park/react'")).toBe(true);
    expect(cardSource.includes('<svg')).toBe(false);
    expect(cardSource.includes('styles.cardSelected')).toBe(false);
    expect(cardSource.includes('workshop')).toBe(false);
    expect(pageSource.includes('<svg')).toBe(false);
    expect(pageSource.includes('workshop')).toBe(false);
  });
});
