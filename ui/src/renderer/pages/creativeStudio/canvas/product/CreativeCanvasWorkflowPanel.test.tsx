/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import {
  createExecutableWorkflowFixture,
  createWorkflowRunFixture,
  IDS,
} from '../../workflows/domain/testFixtures';
import type { CreativeWorkflowRuntimeSnapshot } from '../../workflows/runtime';
import CreativeCanvasWorkflowPanel from './CreativeCanvasWorkflowPanel';

const emptyRuntime: CreativeWorkflowRuntimeSnapshot = {
  loading: false,
  loadError: null,
  runs: [],
  activities: {},
};

describe('CreativeCanvasWorkflowPanel', () => {
  test('renders the canonical workflow catalog and opens a real runner action', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasWorkflowPanel
        workflows={[createExecutableWorkflowFixture()]}
        runtime={emptyRuntime}
        loading={false}
        error={null}
        onRetry={() => undefined}
        onRun={() => undefined}
        onInsertResults={() => undefined}
        onOpenCenter={() => undefined}
      />
    );

    expect(html.includes("data-canvas-product-panel=\"workflows\"")).toBe(true);
    expect(html.includes('Product poster')).toBe(true);
    expect(html.includes('Marketing · 单图')).toBe(true);
    expect(html.includes('运行')).toBe(true);
    expect(html.includes('工作流尚未连接')).toBe(false);
  });

  test('projects a persisted successful run and offers its real result for insertion', () => {
    const run = createWorkflowRunFixture();
    run.record.status = 'succeeded';
    run.record.taskIds = [IDS.task];
    run.record.resultAssetIds = [IDS.asset];
    run.record.queuedAt = 2_100;
    run.record.startedAt = 2_200;
    run.record.completedAt = 2_300;
    const html = renderToStaticMarkup(
      <CreativeCanvasWorkflowPanel
        workflows={[run.workflowSnapshot]}
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
      <CreativeCanvasWorkflowPanel
        workflows={[]}
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
      <CreativeCanvasWorkflowPanel
        workflows={[]}
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
      <CreativeCanvasWorkflowPanel
        workflows={[]}
        runtime={emptyRuntime}
        loading={false}
        error={null}
        onRetry={() => undefined}
        onRun={() => undefined}
        onInsertResults={() => undefined}
        onOpenCenter={() => undefined}
      />
    );

    expect(loading.includes('正在载入工作流')).toBe(true);
    expect(failed.includes('repository unavailable')).toBe(true);
    expect(empty.includes('暂无工作流')).toBe(true);
  });
});
