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
const chromeSource = readFileSync(
  new URL('../chrome/CreativeCanvasChrome.tsx', import.meta.url),
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
const chromeStyle = readFileSync(
  new URL('../chrome/CreativeCanvasChrome.module.css', import.meta.url),
  'utf8'
);

describe('Creative Canvas product route composition', () => {
  test('is a no-props nested route over the canonical Editor store', () => {
    expect(source.includes('useParams<{ canvasId: string }>()')).toBe(true);
    expect(source.includes('useParams<{ projectId: string }>()')).toBe(false);
    expect(source.includes('const projectId = canvasId;')).toBe(true);
    expect(source.includes('data-canvas-id={canvasId}')).toBe(true);
    expect(source.includes('data-project-id={projectId}')).toBe(false);
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
    expect(source.includes('fitCanvasViewport(')).toBe(false);
    expect(source.includes('onFitView={handleFit}')).toBe(false);
    expect(source.match(/onToggleMiniMap=/g)?.length ?? 0).toBe(1);
    expect(source.includes('<CreativeCanvasProductAssetLibrary')).toBe(true);
    expect(source.includes('<CreativeCanvasProductPromptLibrary')).toBe(true);
    expect(source.includes('<CreativeCanvasAgentPanel')).toBe(true);
    expect(source.includes('buildCreativeCanvasAgentContext')).toBe(true);
    expect(source.includes('selectedNodeIds: canvasState.selection.nodeIds')).toBe(true);
    expect(source.includes('planningContext={agentPlanningContext}')).toBe(true);
    expect(source.includes('creativeCanvasAgentOpsPort.apply')).toBe(true);
    expect(source.includes('agentOpsApplyRef.current')).toBe(true);
    expect(source.includes('flushSync(() => setAgentOpsApplyBusy(true))')).toBe(true);
    expect(source.includes('disabled={productDisabled}')).toBe(true);
    expect(source.includes('agentOpsBlockedByCanvasMutation')).toBe(true);
    expect(source.includes('agentOpsReloadRequired')).toBe(true);
    expect(source.includes('agentOpsReloadRequiredRef.current')).toBe(true);
    expect(source.match(/agentOpsReloadRequiredRef\.current/g)?.length ?? 0).toBeGreaterThan(4);
    expect(source.includes('agentPanelRef.current?.refreshAuthority()')).toBe(true);
    expect(source.includes('editor.getSaveState().revision')).toBe(true);
    expect(source.includes('onApplyCanvasOps={handleApplyCanvasAgentOps}')).toBe(true);
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
    expect(source.includes('<CreativeCanvasTemplatePanel')).toBe(true);
    expect(source.includes('<CreativeCanvasTemplateUnwiredPanel')).toBe(false);
    expect(source.includes('<TemplateRunModal')).toBe(true);
    expect(source.includes('templateAssetPicker.pick')).toBe(true);
    expect(source.includes('creativeAssetClient.get(assetId)')).toBe(true);
    expect(source.includes('copyText(selection.prompt)')).toBe(true);
    expect(source.includes('onCopy={handleCopyPrompt}')).toBe(true);
    expect(source.includes('promptInsertTargetNodeId')).toBe(false);
    expect(source.includes('creativeTextNodeFromPrompt')).toBe(false);
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
      '<CreativeCanvasVideoComposer',
      '<CreativeCanvasAudioComposer',
      '<CreativeImageCropDialog',
      '<CreativeImageMaskEditDialog',
      '<CreativeImageSplitDialog',
      '<CanvasImageTaskRuntimeBridge',
      '<CanvasVideoTaskRuntimeBridge',
      '<CanvasAudioTaskRuntimeBridge',
      'prepareCanvasImageCompose',
      'prepareCanvasVideoCompose',
      'canvasVideoComposeMode',
      'prepareCanvasAudioCompose',
      'canvasAudioComposeEligibility',
      'canvasAudioComposeProtocolProfile',
      'confirmCanvasAudioComposeSubmission',
      'orphanCanvasAudioComposeTask',
      'key={`${projectId}:audio:${audioTaskRuntimeEpoch}`}',
      'confirmCanvasVideoComposeSubmission',
      'orphanCanvasVideoComposeTask',
      'videoTaskRuntimeEpoch',
      'key={`${projectId}:video:${videoTaskRuntimeEpoch}`}',
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
    expect(source.includes('creativeStudio.canvas.tasks.recovering')).toBe(true);
    expect(source.includes('正在恢复图片任务…')).toBe(false);
    expect(source.includes('imageWorkbenchModelOptions(modelCatalog, \'image_edit\')')).toBe(true);
    expect(source.includes('withCanvasImageComposeDraft(node, update(current))')).toBe(true);
    expect(source.includes('mergeKey: `image-composer:${nodeId}`')).toBe(true);
    expect(source.includes('imageComposeDrafts')).toBe(false);
    expect(source.includes("if (state && target?.type === 'image')")).toBe(false);
    expect(source.includes('target.data.assetId)')).toBe(false);
    expect(
      editorStyle.includes('[data-canvas-video-composer]')
    ).toBe(true);
    expect(
      editorStyle.includes('[data-canvas-audio-composer]')
    ).toBe(true);
  });

  test('documents the exact lazy Router handoff without modifying Router here', () => {
    expect(
      wiring.includes("import('@renderer/pages/creativeStudio/canvas/product')")
    ).toBe(true);
    expect(wiring.includes('path="canvas/:canvasId"')).toBe(true);
    expect(wiring.includes('path="canvas/:projectId"')).toBe(false);
    expect(wiring.includes('/workshop/canvases')).toBe(true);
    expect(wiring.includes('/api/creative-studio/canvases/{canvasId}/agent-ops')).toBe(
      true
    );
    expect(wiring.includes('CanvasNode { canvasId, nodeId }')).toBe(true);
    expect(wiring.includes('canvas_id, node_id')).toBe(true);
    expect(wiring.includes('## Compatibility only')).toBe(true);
    expect(wiring.includes('image_edit` / `i2i')).toBe(true);
    expect(wiring.includes('authoritative 404')).toBe(true);
    expect(wiring.includes('nomifun.creative-studio.canvas-ops/v1')).toBe(true);
    expect(wiring.includes('应用到画布')).toBe(true);
    expect(wiring.includes('automatically retries a response-loss mutation')).toBe(true);
  });

  test('derives the focused canvas palette from the active app theme', () => {
    expect(style.includes('--creative-canvas-surface-background: var(--color-bg-1)')).toBe(
      true
    );
    expect(style.includes('--creative-canvas-node-fill: color-mix(')).toBe(true);
    expect(style.includes('--color-text-1: #292524')).toBe(false);
    expect(style.includes('--primary-6: 87, 83, 78')).toBe(false);
    expect(style.includes('color-scheme: inherit')).toBe(true);
    expect(style.includes(":global([data-theme='dark']) .root")).toBe(true);
    expect(style.includes('--creative-canvas-grid-line: color-mix(')).toBe(true);
  });

  test('keeps the bottom icon toolbar horizontally centered on wide canvases', () => {
    expect(style.includes('transform: translateY(-4px)')).toBe(true);
    expect(style.includes('translate(clamp(0px, 10vw, 142px), -4px)')).toBe(false);
    expect(/\.toolbarButton > button\s*\{[\s\S]*?width:\s*100%;[\s\S]*?height:\s*100%;[\s\S]*?place-items:\s*center;/.test(style)).toBe(true);
    expect(/\.iconButton > button\s*\{[\s\S]*?width:\s*100%;[\s\S]*?height:\s*100%;[\s\S]*?place-items:\s*center;/.test(chromeStyle)).toBe(true);
    expect(/\.toolbarButton :global\(\.i-icon\)\s*\{[\s\S]*?width:\s*17px;[\s\S]*?height:\s*17px;[\s\S]*?line-height:\s*0;/.test(style)).toBe(true);
    expect(/\.iconButton :global\(\.i-icon\)\s*\{[\s\S]*?width:\s*17px;[\s\S]*?height:\s*17px;[\s\S]*?line-height:\s*0;/.test(chromeStyle)).toBe(true);
    expect(source.includes('strokeWidth: 3')).toBe(true);
    expect(chromeSource.includes('strokeWidth: 3')).toBe(true);
    expect(style.includes('opacity: 0.52')).toBe(true);
    expect(chromeStyle.includes('opacity: 0.52')).toBe(true);
  });
});
