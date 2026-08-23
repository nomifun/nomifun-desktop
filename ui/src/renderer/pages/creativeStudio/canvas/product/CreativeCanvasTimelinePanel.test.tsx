/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';

import type { CreativeCanvasNode } from '../../domain';
import { createInitialCanvasState } from '../core';
import CreativeCanvasTimelinePanel, {
  creativeCanvasDirectorNodes,
  formatCreativeDirectorTime,
  projectCreativeDirectorTimeline,
} from './CreativeCanvasTimelinePanel';

const director = (
  overrides: Partial<Extract<CreativeCanvasNode, { type: 'director' }>['data']> = {},
  id = '0190f5fe-7c00-7a00-8000-000000000201'
): Extract<CreativeCanvasNode, { type: 'director' }> => ({
  id,
  type: 'director',
  position: { x: 10, y: 20 },
  size: { width: 440, height: 280 },
  groupId: null,
  zIndex: 0,
  locked: false,
  data: {
    sceneId: null,
    cameraId: null,
    timelineMs: 0,
    durationMs: 0,
    ...overrides,
  },
});

const noop = () => undefined;

describe('CreativeCanvasTimelinePanel', () => {
  test('shows an honest add action when no canonical director node exists', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasTimelinePanel
        state={createInitialCanvasState()}
        onSelectNode={noop}
        onAddDirector={noop}
        onOpenDirector={noop}
      />
    );

    expect(html.includes('data-director-timeline-state="empty"')).toBe(true);
    expect(html.includes('还没有导演场景')).toBe(true);
    expect(html.includes('添加导演节点')).toBe(true);
    expect(html.includes('示例')).toBe(false);
  });

  test('projects the one real director pointer, camera, and read-only time', () => {
    const node = director({
      sceneId: '0190f5fe-7c00-7a00-8000-000000000211',
      cameraId: '0190f5fe-7c00-7a00-8000-000000000212',
      timelineMs: 65_432,
      durationMs: 125_000,
    });
    const html = renderToStaticMarkup(
      <CreativeCanvasTimelinePanel
        state={createInitialCanvasState({
          document: { nodes: [node], connections: [] },
        })}
        onSelectNode={noop}
        onAddDirector={noop}
        onOpenDirector={noop}
      />
    );

    expect(html.includes('data-director-timeline-state="ready"')).toBe(true);
    expect(html.includes(`data-director-node-id="${node.id}"`)).toBe(true);
    expect(html.includes('场景已连接')).toBe(true);
    expect(html.includes(node.data.sceneId!)).toBe(true);
    expect(html.includes(node.data.cameraId!)).toBe(true);
    expect(html.includes('01:05.432')).toBe(true);
    expect(html.includes('02:05.000')).toBe(true);
    expect(html.includes('aria-label="导演时间线进度"')).toBe(true);
    expect(html.includes('打开 3D 导演台')).toBe(true);
    expect(html.includes('画布仅显示已保存的 canonical 投影')).toBe(true);
    expect(html.includes('type="range"')).toBe(false);
  });

  test('fails closed and exposes exact nodes when legacy data contains multiple directors', () => {
    const first = director();
    const second = director({}, '0190f5fe-7c00-7a00-8000-000000000202');
    const state = createInitialCanvasState({
      document: { nodes: [first, second], connections: [] },
    });
    const html = renderToStaticMarkup(
      <CreativeCanvasTimelinePanel state={state} onSelectNode={noop} onAddDirector={noop} onOpenDirector={noop} />
    );

    expect(creativeCanvasDirectorNodes(state).map((node) => node.id)).toEqual([first.id, second.id]);
    expect(html.includes('data-director-timeline-state="conflict"')).toBe(true);
    expect(html.includes('检测到多个导演节点')).toBe(true);
    expect(html.includes(first.id)).toBe(true);
    expect(html.includes(second.id)).toBe(true);
    expect(html.includes('打开 3D 导演台')).toBe(false);
  });

  test('normalizes invalid projection values without inventing duration', () => {
    const node = director({ timelineMs: 2_500, durationMs: 1_000 });
    expect(projectCreativeDirectorTimeline(node)).toEqual({
      currentMs: 1_000,
      durationMs: 1_000,
      progress: 1,
    });
    expect(projectCreativeDirectorTimeline(director({ timelineMs: 80, durationMs: 0 }))).toEqual({
      currentMs: 0,
      durationMs: 0,
      progress: 0,
    });
    expect(formatCreativeDirectorTime(3_725_007)).toBe('62:05.007');
    expect(formatCreativeDirectorTime(Number.NaN)).toBe('00:00.000');
  });
});
