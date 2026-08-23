/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';

import { StandaloneHistoryRetireDialog } from './shared';

describe('standalone history retirement dialog', () => {
  test('explains non-destructive retirement and requires explicit confirmation', () => {
    const html = renderToStaticMarkup(
      <StandaloneHistoryRetireDialog
        open
        count={2}
        busy={false}
        onCancel={() => undefined}
        onConfirm={() => undefined}
      />
    );
    expect(html.includes('role="dialog"')).toBe(true);
    expect(html.includes('从历史移除 2 条')).toBe(true);
    expect(html.includes('任务审计、输入素材和生成结果会继续安全保留')).toBe(true);
    expect(html.includes('>取消<')).toBe(true);
    expect(html.includes('>从历史移除<')).toBe(true);
  });

  test('renders nothing while closed', () => {
    const html = renderToStaticMarkup(
      <StandaloneHistoryRetireDialog
        open={false}
        count={0}
        busy={false}
        onCancel={() => undefined}
        onConfirm={() => undefined}
      />
    );
    expect(html).toBe('');
  });
});
