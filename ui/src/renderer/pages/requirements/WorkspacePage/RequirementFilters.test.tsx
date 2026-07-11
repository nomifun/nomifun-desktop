import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import { FilterTrigger } from './RequirementFilters';

describe('RequirementFilters trigger', () => {
  test('renders icon, function label, and selected content', () => {
    const html = renderToStaticMarkup(<FilterTrigger icon={<span>icon</span>} label='标签' value='产品' />);

    expect(html.includes('icon')).toBe(true);
    expect(html.includes('标签')).toBe(true);
    expect(html.includes('产品')).toBe(true);
    expect(html.includes('aria-label="标签: 产品"')).toBe(true);
  });

  test('omits selected content when the filter is inactive', () => {
    const html = renderToStaticMarkup(<FilterTrigger icon={<span>icon</span>} label='状态' />);

    expect(html.includes('aria-label="状态"')).toBe(true);
    expect(html.includes('undefined')).toBe(false);
  });
});
