/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(
  new URL('./CreativeCanvasProductRoute.tsx', import.meta.url),
  'utf8'
);
const wiring = readFileSync(new URL('./WIRING.md', import.meta.url), 'utf8');
const style = readFileSync(
  new URL('./CreativeCanvasProductRoute.module.css', import.meta.url),
  'utf8'
);
const editorStyle = readFileSync(
  new URL('../editor/CreativeCanvasEditor.module.css', import.meta.url),
  'utf8'
);

describe('Creative Canvas product route composition', () => {
  test('is a no-props nested route over the canonical Editor store', () => {
    expect(source.includes('useParams<{ projectId: string }>()')).toBe(true);
    expect(source.includes('projectId={projectId}')).toBe(true);
    expect(source.includes('<CreativeCanvasChrome')).toBe(true);
    expect(source.includes('<CreativeCanvasEditor')).toBe(true);
    expect(source.includes('showSaveState={false}')).toBe(true);
    expect(source.includes('editorRef.current?.dispatch')).toBe(true);
    expect(source.includes('editorRef.current?.setPanels')).toBe(true);
    expect(source.includes('creativeCanvasProductPanelViews(panels)')).toBe(
      true
    );
    expect(source.includes('withCreativeCanvasRightView')).toBe(true);
    expect(source.includes('canvasCommands.updateNode(node')).toBe(true);
    expect(source.includes('onUpdateNode={handleUpdateNode}')).toBe(true);
    expect(source.includes('useReducer(')).toBe(false);
    expect(source.includes('canvasReducer(')).toBe(false);
    expect(source.includes('export default CreativeCanvasProductRoute')).toBe(
      true
    );
  });

  test('composes contained nodes, per-edge renderer, minimap, and real libraries', () => {
    expect(source.includes('placement="contained"')).toBe(true);
    expect(
      source.includes('<CreativeCanvasConnectionEdge {...context} />')
    ).toBe(true);
    expect(source.includes('<CanvasMiniMap')).toBe(true);
    expect(source.includes('fitCanvasViewport(')).toBe(true);
    expect(source.includes('<CreativeCanvasProductAssetLibrary')).toBe(true);
    expect(source.includes('<CreativeCanvasProductPromptLibrary')).toBe(true);
    expect(source.includes('<CreativeCanvasAgentPanel')).toBe(true);
    expect(
      source.includes('onAgentSessionsChange={handleAgentSessionsChange}')
    ).toBe(true);
    expect(
      source.includes('persistAgentSessions(sessions, activeSessionId)')
    ).toBe(true);
    expect(source.includes('agentPanelRef.current?.prepareToLeave()')).toBe(
      true
    );
    expect(source.includes('<CreativeCanvasAssistantUnwiredPanel')).toBe(false);
    expect(source.includes('<CreativeCanvasWorkflowPanel')).toBe(true);
    expect(source.includes('<CreativeCanvasWorkflowUnwiredPanel')).toBe(false);
    expect(source.includes('<WorkflowRunModal')).toBe(true);
    expect(source.includes('workflowAssetPicker.pick')).toBe(true);
    expect(source.includes('creativeAssetClient.get(assetId)')).toBe(true);
    expect(source.includes('<CreativeCanvasTimelinePanel')).toBe(true);
    expect(source.includes('<CreativeCanvasTimelineUnwiredPanel')).toBe(false);
    expect(source.includes("onAddDirector={() => addNode('director')}")).toBe(
      true
    );
    expect(source.includes('handleOpenDirector(nodeId)')).toBe(true);
  });

  test('resolves typed interaction intents through real product boundaries', () => {
    for (const token of [
      'onIntegrationIntent',
      '<CreativeCanvasInteractionOverlays',
      'resolveCanvasContextAction',
      'validateCanvasConnection',
      'creativeAssetClient.upload',
      'uploadCanvasImageNodeAsset',
      'fillEmptyCanvasImageNodeFromAsset',
      'imageNodeUploadInputRef',
      'visible={Boolean(singleSelected)}',
      'hasImageContent={Boolean(node.data.assetId)}',
      'navigator.clipboard.read()',
      'manualUploadRejectionMessage',
      'pendingPanoramaChoice',
      '<CreativeCanvasImageToolbar',
      '<CreativeCanvasImageComposer',
      '<CreativeImageCropDialog',
      '<CreativeImageMaskEditDialog',
      '<CreativeImageSplitDialog',
      '<CanvasImageTaskRuntimeBridge',
      'prepareCanvasImageCompose',
      'CREATIVE_IMAGE_COMPOSE_OPERATION',
      'buildCreativeImageMaskReference',
      'uploadCreativeImageMaskReference',
      'prepareCanvasImageMaskEdit',
      'orphanCanvasImageMaskEditTask',
      'cropCreativeImageAsset',
      'splitCreativeImageAsset',
      'uploadCreativeImageCrop',
      'uploadCreativeImageSplit',
      'removeUploadedCreativeImageSplit',
      'nextDerivedImagePosition',
      'createCreativeImageSplitCanvasLayout',
      'creativeImageSplitNodePosition',
      'canvasCommands.connect(source.id, derived.id',
      'const flush = await editor.flush()',
      'imageToolBusyRef.current',
      'creativeStudioDirectorProjectPath(projectId)',
      "state.document.nodes.filter((node) => node.type === 'director')",
      "handleBottomViewChange('timeline')",
      'onOpen={onOpen}',
      'onToggleLock={onToggleLock}',
    ]) {
      expect(source.includes(token)).toBe(true);
    }
    expect(source.includes('URL.createObjectURL')).toBe(false);
    expect(source.includes('data:image/')).toBe(false);
  });

  test('uses CAS recovery and delegates generation to the typed runtime gateway', () => {
    expect(source.includes('await editor.flush()')).toBe(true);
    expect(source.includes('creativeCanvasBlockedLeaveMessage(result)')).toBe(
      true
    );
    expect(source.includes('await editorRef.current.reloadRemote()')).toBe(
      true
    );
    expect(source.includes('setBackground(next)')).toBe(true);
    expect(source.includes('creativeCanvasSaveDisplayMessage(save)')).toBe(true);
    expect(source.includes('save.error?.message ?? notice')).toBe(false);
    expect(source.includes('localStorage')).toBe(false);
    expect(source.includes('sessionStorage')).toBe(false);
    expect(source.includes('fetch(')).toBe(false);
    expect(source.includes('Math.random')).toBe(false);
    expect(source.includes('runtime.submit(prepared.plan)')).toBe(true);
    expect(source.includes('runtime.retrySubmission(')).toBe(true);
    expect(source.includes('runtime.taskExists(')).toBe(true);
    expect(source.includes('imageWorkbenchModelOptions(modelCatalog, \'image_edit\')')).toBe(true);
    expect(source.includes('withCanvasImageComposeDraft(node, update(current))')).toBe(true);
    expect(source.includes('mergeKey: `image-composer:${nodeId}`')).toBe(true);
    expect(source.includes('imageComposeDrafts')).toBe(false);
    expect(source.includes("if (state && target?.type === 'image')")).toBe(true);
    expect(source.includes('target.data.assetId)')).toBe(false);
    expect(
      editorStyle.includes('.nodePlacement:has([data-canvas-image-composer])')
    ).toBe(true);
  });

  test('documents the exact lazy Router handoff without modifying Router here', () => {
    expect(
      wiring.includes("import('@renderer/pages/creativeStudio/canvas/product')")
    ).toBe(true);
    expect(wiring.includes('path="canvas/:projectId"')).toBe(true);
    expect(wiring.includes('CREATIVE_STUDIO_PROJECTS_PATH')).toBe(true);
    expect(wiring.includes('image_edit` / `i2i')).toBe(true);
    expect(wiring.includes('authoritative 404')).toBe(true);
  });

  test('keeps the focused reference palette independent from the app theme', () => {
    expect(style.includes('--creative-canvas-surface-background: #f4f2ed')).toBe(
      true
    );
    expect(style.includes('--creative-canvas-node-fill: #e7e5df')).toBe(true);
    expect(style.includes('--color-text-1: #292524')).toBe(true);
    expect(style.includes('--primary-6: 87, 83, 78')).toBe(true);
    expect(style.includes('color-scheme: light')).toBe(true);
    expect(style.includes("[data-theme='dark']")).toBe(false);
  });
});
