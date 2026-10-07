import { afterEach, describe, expect, test } from 'bun:test';
import { cleanup, render } from '@testing-library/react';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import NomiPagination, { NOMI_PAGINATION_CLASS_NAME } from './NomiPagination';

afterEach(cleanup);

describe('NomiPagination', () => {
  test('always applies the shared pagination contract while preserving host classes', () => {
    const view = render(
      <NomiPagination
        className='host-pagination'
        current={2}
        pageSize={10}
        total={30}
        showTotal
        onChange={() => undefined}
      />
    );

    const root = view.container.querySelector('.arco-pagination');
    expect(root).not.toBeNull();
    expect(root?.classList.contains(NOMI_PAGINATION_CLASS_NAME)).toBe(true);
    expect(root?.classList.contains('host-pagination')).toBe(true);
    expect(root?.classList.contains('arco-pagination-size-small')).toBe(false);
    expect(root?.querySelector('.arco-pagination-item-active')?.textContent).toBe('2');
  });

  test('is the only renderer component allowed to import Arco Pagination directly', () => {
    const baseDirectory = dirname(fileURLToPath(import.meta.url));
    const rendererRoot = resolve(baseDirectory, '..', '..');
    const directImports: string[] = [];

    const visit = (directory: string) => {
      for (const entry of readdirSync(directory, { withFileTypes: true })) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) visit(path);
        else if (entry.name.endsWith('.tsx')) {
          const source = readFileSync(path, 'utf8');
          if (/import\s*\{[^}]*\bPagination\b[^}]*\}\s*from\s*['"]@arco-design\/web-react['"]/s.test(source)) {
            directImports.push(path);
          }
        }
      }
    };

    visit(rendererRoot);
    expect(directImports).toEqual([resolve(baseDirectory, 'NomiPagination.tsx')]);
  });
});
