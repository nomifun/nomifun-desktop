/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import DirectorWorkbenchShell from './DirectorWorkbenchShell';
import type {
  DirectorCameraInspectorValue,
  DirectorInspectorValue,
  DirectorWorkbenchShellProps,
} from './types';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
  interpolation: { escapeValue: false },
});

const environmentInspector: DirectorInspectorValue = {
  kind: 'environment',
  sceneScale: 1,
  position: { x: 0, y: 0, z: 0 },
  rotation: { x: 0, y: 0, z: 0 },
  panorama: null,
  skyColor: '#000000',
  panoramaYaw: 0,
  panoramaRadius: 60,
  showLabels: true,
  snapToGrid: false,
  showGround: true,
  showGrid: true,
  groundHeight: 0,
  groundOpacity: 0.7,
};

const baseProps: DirectorWorkbenchShellProps = {
  viewMode: 'director',
  transformMode: 'translate',
  viewportSlot: <div data-real-viewport-mount='three-renderer' />,
  viewportOverlaySlot: <div data-real-label-overlay='true'>角色01</div>,
  gizmoSlot: <div data-native-gizmo-mount='true' />,
  sceneQuery: '',
  sceneGroups: [
    {
      id: 'characters',
      label: '角色',
      objects: [
        {
          id: 'character-01',
          name: '角色01',
          kind: 'character',
          visible: true,
          locked: false,
          selected: true,
        },
      ],
    },
    {
      id: 'cameras',
      label: '摄像机',
      objects: [
        {
          id: 'camera-01',
          name: '机位01',
          kind: 'camera',
          visible: true,
          locked: false,
        },
      ],
    },
  ],
  inspector: environmentInspector,
  bodyTypeOptions: [
    { value: 'mannequin', label: '标准角色' },
    { value: 'female', label: '女性角色' },
  ],
  posePresetOptions: [
    { value: 'idle', label: '站立' },
    { value: 'walk', label: '行走' },
  ],
  modelLibraryOpen: true,
  modelLibraryItems: [
    {
      id: 'model-01',
      name: '摄影棚座椅',
      thumbnailUrl: 'nomifun-asset://model-01/thumbnail',
      deletable: true,
    },
  ],
  aspectPickerOpen: true,
  aspectRatio: '16:9',
  showRuleOfThirds: true,
  panelsCollapsed: false,
  timeline: {
    open: true,
    height: 240,
    currentTimeSeconds: 1.25,
    durationSeconds: 8,
    fps: 30,
    playing: false,
    loop: true,
    autoKey: true,
    selectedTrackId: 'character-track',
    selectedKeyframeId: 'character-keyframe-01',
    tracks: [
      {
        id: 'character-track',
        label: '角色01 · 位置',
        kind: 'character',
        selected: true,
        keyframes: [
          { id: 'character-keyframe-01', timeSeconds: 0, selected: true },
          { id: 'character-keyframe-02', timeSeconds: 4 },
        ],
      },
      {
        id: 'camera-track',
        label: '机位01 · FOV',
        kind: 'camera',
        keyframes: [{ id: 'camera-keyframe-01', timeSeconds: 2 }],
      },
    ],
  },
  onViewModeChange: () => undefined,
  onTransformModeChange: () => undefined,
  onSceneQueryChange: () => undefined,
  onSceneObjectSelect: () => undefined,
  onSceneObjectVisibilityChange: () => undefined,
  onSceneObjectLockChange: () => undefined,
  onInspectorChange: () => undefined,
  onChoosePanorama: () => undefined,
  onRemovePanorama: () => undefined,
  onReimportObjectModel: () => undefined,
  onPosePresetSelect: () => undefined,
  onCameraCapture: () => undefined,
  onCaptureView: () => undefined,
  onCaptureDelete: () => undefined,
  onCaptureSendToCanvas: () => undefined,
  onCaptureClearAll: () => undefined,
  onCaptureSendAll: () => undefined,
  onAddCharacter: () => undefined,
  onImportPanorama: () => undefined,
  onImportModel: () => undefined,
  onAddCamera: () => undefined,
  onCaptureViewport: () => undefined,
  onModelLibraryOpenChange: () => undefined,
  onModelLibraryAdd: () => undefined,
  onModelLibraryDelete: () => undefined,
  onAspectPickerOpenChange: () => undefined,
  onAspectRatioChange: () => undefined,
  onRuleOfThirdsChange: () => undefined,
  onPanelsCollapsedChange: () => undefined,
  onTimelineOpenChange: () => undefined,
  onTimelinePlayingChange: () => undefined,
  onTimelineLoopChange: () => undefined,
  onTimelineAutoKeyChange: () => undefined,
  onTimelineTimeChange: () => undefined,
  onTimelineDurationChange: () => undefined,
  onTimelineTrackSelect: () => undefined,
  onKeyframeSelect: () => undefined,
  onKeyframeAdd: () => undefined,
  onKeyframeDelete: () => undefined,
  onTimelineExport: () => undefined,
};

const renderShell = (overrides: Partial<DirectorWorkbenchShellProps> = {}) =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <DirectorWorkbenchShell {...baseProps} {...overrides} />
    </I18nextProvider>
  );

describe('DirectorWorkbenchShell presentation', () => {
  test('renders the source-density shell around caller-owned viewport and gizmo slots', () => {
    const html = renderShell();

    expect(html.includes('data-director-workbench="true"')).toBe(true);
    expect(html.includes('data-real-viewport-mount="three-renderer"')).toBe(true);
    expect(html.includes('data-native-gizmo-mount="true"')).toBe(true);
    expect(html.includes('data-real-label-overlay="true"')).toBe(true);
    expect(html.includes('3D导演台')).toBe(true);
    expect(html.includes('导演视角')).toBe(true);
    expect(html.includes('机位视角')).toBe(true);
    expect(html.includes('3D视口快捷工具')).toBe(true);
    expect(html.includes('添加角色')).toBe(true);
    expect(html.includes('导入全景图')).toBe(true);
    expect(html.includes('导入本地模型')).toBe(true);
    expect(html.includes('当前视角截图')).toBe(true);
  });

  test('renders scene hierarchy, environment inspector and controlled composition overlays', () => {
    const html = renderShell();

    expect(html.includes('data-director-scene-sidebar="true"')).toBe(true);
    expect(html.includes('角色01')).toBe(true);
    expect(html.includes('机位01')).toBe(true);
    expect(html.includes('data-director-inspector="environment"')).toBe(true);
    expect(html.includes('场景缩放')).toBe(true);
    expect(html.includes('全景背景')).toBe(true);
    expect(html.includes('未连接全景图')).toBe(true);
    expect(html.includes('角色标签')).toBe(true);
    expect(html.includes('data-aspect-ratio="16:9"')).toBe(true);
    expect(html.includes('data-rule-of-thirds="true"')).toBe(true);
  });

  test('preserves panorama names and removal controls for both available and deleted covers', () => {
    const panorama = {
      assetId: 'panorama-01',
      name: '海边全景',
      thumbnailUrl: '/panorama-thumbnail.jpg',
    };
    const available = renderShell({ inspector: { ...environmentInspector, panorama } });
    expect(available.includes('src="/panorama-thumbnail.jpg"')).toBe(true);
    expect(available.includes('alt="海边全景 全景图缩略图"')).toBe(true);
    expect(available.includes('aria-label="删除全景图"')).toBe(true);

    const deleted = renderShell({
      inspector: { ...environmentInspector, panorama: { ...panorama, availability: 'deleted' } },
    });
    expect(deleted.includes('data-asset-media-state="deleted"')).toBe(true);
    expect(deleted.includes('/panorama-thumbnail.jpg')).toBe(false);
    expect(deleted.includes('海边全景')).toBe(true);
    expect(deleted.includes('aria-label="删除全景图"')).toBe(true);
  });

  test('renders real model thumbnails and timeline keyframes without inventing 3D media', () => {
    const html = renderShell();

    expect(html.includes('aria-label="模型库"')).toBe(true);
    expect(html.includes('nomifun-asset://model-01/thumbnail')).toBe(true);
    expect(html.includes('摄影棚座椅')).toBe(true);
    expect(html.includes('data-director-timeline="true"')).toBe(true);
    expect(html.includes('角色01 · 位置')).toBe(true);
    expect(html.includes('机位01 · FOV')).toBe(true);
    expect(html.includes('0.00 秒关键帧')).toBe(true);
    expect(html.includes('<iframe')).toBe(false);
    expect(html.includes('<canvas')).toBe(false);
    expect(html.includes('.glb')).toBe(false);
    expect(html.includes('blob:')).toBe(false);
  });

  test('renders camera properties and the real capture list as controlled inspector states', () => {
    const cameraInspector: DirectorCameraInspectorValue = {
      kind: 'camera',
      id: 'camera-01',
      name: '机位01',
      position: { x: 1, y: 2, z: 5 },
      rotation: { x: 0, y: 18, z: 0 },
      fov: 45,
      tab: 'captures',
      captures: [
        {
          id: 'capture-01',
          assetId: 'asset-capture-01',
          name: '机位01截图01',
          thumbnailUrl: 'nomifun-asset://capture-01/thumbnail',
          imageUrl: 'nomifun-asset://capture-01/original',
          cameraId: 'camera-01',
        },
      ],
    };
    const html = renderShell({ inspector: cameraInspector });

    expect(html.includes('data-director-inspector="camera"')).toBe(true);
    expect(html.includes('data-director-capture-list="true"')).toBe(true);
    expect(html.includes('当前机位截图')).toBe(true);
    expect(html.includes('nomifun-asset://capture-01/thumbnail')).toBe(true);
    expect(html.includes('alt="机位01截图01 缩略图"')).toBe(true);
    expect(html.includes('查看截图 机位01截图01')).toBe(true);
    expect(html.includes('发送到画布 机位01截图01')).toBe(true);
    expect(html.includes('删除截图 机位01截图01')).toBe(true);

    const deleted = renderShell({ inspector: {
      ...cameraInspector,
      captures: cameraInspector.captures.map((capture) => ({ ...capture, availability: 'deleted' })),
    } });
    expect(deleted.includes('data-asset-media-state="deleted"')).toBe(true);
    expect(deleted.includes('nomifun-asset://capture-01/thumbnail')).toBe(false);
    expect(deleted.includes('nomifun-asset://capture-01/original')).toBe(false);
    expect(deleted.includes('删除截图 机位01截图01')).toBe(true);
  });

  test('removes both side panels in controlled full-viewport mode', () => {
    const html = renderShell({ panelsCollapsed: true });

    expect(html.includes('data-panels-collapsed="true"')).toBe(true);
    expect(html.includes('data-director-scene-sidebar')).toBe(false);
    expect(html.includes('data-director-inspector')).toBe(false);
    expect(html.includes('data-real-viewport-mount="three-renderer"')).toBe(true);
  });
});
