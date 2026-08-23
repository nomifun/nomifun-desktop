/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';

import {
  createExecutableTemplateFixture,
  createTemplateRunFixture,
  IDS,
} from '../../templates/domain/testFixtures';
import type { CreativeTemplateRuntimeSnapshot } from '../../templates/runtime';
import CreativeCanvasTemplatePanel from './CreativeCanvasTemplatePanel';

const emptyRuntime: CreativeTemplateRuntimeSnapshot = {
  loading: false,
  loadError: null,
  runs: [],
  activities: {},
};

describe('CreativeCanvasTemplatePanel', () => {
  test('renders the canonical template catalog and opens a real runner action', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasTemplatePanel
        templates={[createExecutableTemplateFixture()]}
        runtime={emptyRuntime}
        loading={false}
        error={null}
        onRetry={() => undefined}
        onRun={() => undefined}
        onInsertResults={() => undefined}
        onOpenCenter={() => undefined}
      />
    );

    expect(html.includes("data-canvas-product-panel=\"templates\"")).toBe(true);
    expect(html.includes('Product poster')).toBe(true);
    expect(html.includes('Marketing · 单图')).toBe(true);
    expect(html.includes('运行')).toBe(true);
    expect(html.includes('模板尚未连接')).toBe(false);
  });

  test('projects a persisted successful run and offers its real result for insertion', () => {
    const run = createTemplateRunFixture();
    run.record.status = 'succeeded';
    run.record.taskIds = [IDS.task];
    run.record.resultAssetIds = [IDS.asset];
    run.record.queuedAt = 2_100;
    run.record.startedAt = 2_200;
    run.record.completedAt = 2_300;
    const html = renderToStaticMarkup(
      <CreativeCanvasTemplatePanel
        templates={[run.templateSnapshot]}
        runtime={{ ...emptyRuntime, runs: [run] }}
        loading={false}
        error={null}
        onRetry={() => undefined}
        onRun={() => undefined}
        onInsertResults={() => undefined}
        onOpenCenter={() => undefined}
      />
    );

    expect(html.includes('已完成')).toBe(true);
    expect(html.includes('1 项真实结果')).toBe(true);
    expect(html.includes('插入结果')).toBe(true);
  });

  test('keeps loading, error and empty repository states explicit', () => {
    const loading = renderToStaticMarkup(
      <CreativeCanvasTemplatePanel
        templates={[]}
        runtime={emptyRuntime}
        loading
        error={null}
        onRetry={() => undefined}
        onRun={() => undefined}
        onInsertResults={() => undefined}
        onOpenCenter={() => undefined}
      />
    );
    const failed = renderToStaticMarkup(
      <CreativeCanvasTemplatePanel
        templates={[]}
        runtime={emptyRuntime}
        loading={false}
        error='repository unavailable'
        onRetry={() => undefined}
        onRun={() => undefined}
        onInsertResults={() => undefined}
        onOpenCenter={() => undefined}
      />
    );
    const empty = renderToStaticMarkup(
      <CreativeCanvasTemplatePanel
        templates={[]}
        runtime={emptyRuntime}
        loading={false}
        error={null}
        onRetry={() => undefined}
        onRun={() => undefined}
        onInsertResults={() => undefined}
        onOpenCenter={() => undefined}
      />
    );

    expect(loading.includes('正在载入模板')).toBe(true);
    expect(failed.includes('repository unavailable')).toBe(true);
    expect(empty.includes('暂无模板')).toBe(true);
    expect(empty.includes('打开模板工作台')).toBe(true);
  });
});
