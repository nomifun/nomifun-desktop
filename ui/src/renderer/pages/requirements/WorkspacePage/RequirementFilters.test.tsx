import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import { FilterTrigger } from './RequirementFilters';

describe('RequirementFilters trigger', () => {
  test('forwards a DOM ref so Arco can anchor the popup', () => {
    expect((FilterTrigger as unknown as { $$typeof?: symbol }).$$typeof).toBe(Symbol.for('react.forward_ref'));
  });

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

  test('uses the primary active color when selected or open', () => {
    const html = renderToStaticMarkup(
      <FilterTrigger icon={<span>icon</span>} label='标签' value='产品' active />
    );

    expect(html.includes('aria-pressed="true"')).toBe(true);
    expect(html.includes('!bg-primary-1')).toBe(true);
    expect(html.includes('!text-primary-6')).toBe(true);
  });
});
