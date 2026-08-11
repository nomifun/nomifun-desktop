/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import {
  getMarkdownInternalRoute,
  IMAGE_MODEL_MANAGEMENT_MARKDOWN_LINK,
  IMAGE_MODEL_MANAGEMENT_ROUTE,
} from './markdownLinkPolicy';

const markdownSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');

describe('Markdown internal link policy', () => {
  test('allows only the exact image model-management link', () => {
    expect(getMarkdownInternalRoute(IMAGE_MODEL_MANAGEMENT_MARKDOWN_LINK)).toBe(
      IMAGE_MODEL_MANAGEMENT_ROUTE
    );

    for (const href of [
      'nomifun://model-management/image/',
      'nomifun://model-management/image?source=agent',
      'nomifun://model-management/image#configure',
      'NOMIFUN://model-management/image',
      'nomifun://model-management/video',
      'nomifun://settings',
      '/models?section=image',
      'https://nomifun.com/models?section=image',
    ]) {
      expect(getMarkdownInternalRoute(href)).toBeUndefined();
    }
  });

  test('routes the allowlisted target internally and keeps the external fallback', () => {
    expect(markdownSource.includes('getMarkdownInternalRoute(url)')).toBe(true);
    expect(markdownSource.includes('const internalRoute = getMarkdownInternalRoute(href);')).toBe(true);
    expect(markdownSource.includes('void navigate(internalRoute);')).toBe(true);
    expect(markdownSource.includes('href={internalRoute ?? href}')).toBe(true);
    expect(markdownSource.includes("target={internalRoute ? undefined : '_blank'}")).toBe(true);
    expect(markdownSource.includes('openExternalUrl(externalHref).catch')).toBe(true);
  });
});
