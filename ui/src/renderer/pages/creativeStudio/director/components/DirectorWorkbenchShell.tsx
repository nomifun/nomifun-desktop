/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Close } from '@icon-park/react';
import { Button, Tooltip } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import DirectorInspector from './DirectorInspector';
import DirectorSceneSidebar from './DirectorSceneSidebar';
import DirectorTimeline from './DirectorTimeline';
import DirectorViewport from './DirectorViewport';
import styles from './DirectorWorkbenchShell.module.css';
import type { DirectorViewMode, DirectorWorkbenchShellProps } from './types';

const DirectorWorkbenchShell: React.FC<DirectorWorkbenchShellProps> = (props) => {
  const { t } = useTranslation();
  const timelineHeight = Math.min(520, Math.max(180, props.timeline.height));
  const shellStyle = {
    '--director-timeline-height': `${timelineHeight}px`,
  } as React.CSSProperties;

  return (
    <section
      className={styles.shell}
      style={shellStyle}
      data-director-workbench
      data-view-mode={props.viewMode}
      data-panels-collapsed={props.panelsCollapsed}
      data-timeline-open={props.timeline.open}
    >
      <header className={styles.topBar}>
        <div className={styles.topBarLeft}>
          <h1>
            {props.title ||
              t('creativeStudio.director.workbench.title', {
                defaultValue: '3D导演台',
              })}
          </h1>
        </div>

        <div
          className={styles.viewModeToggle}
          role='group'
          aria-label={t('creativeStudio.director.workbench.viewMode.label', {
            defaultValue: '视角切换',
          })}
        >
          {(['director', 'camera'] as const).map((mode: DirectorViewMode) => (
            <Button
              key={mode}
              type={props.viewMode === mode ? 'primary' : 'text'}
              aria-pressed={props.viewMode === mode}
              disabled={props.disabled}
              onClick={() => props.onViewModeChange(mode)}
            >
              {mode === 'director'
                ? t('creativeStudio.director.workbench.viewMode.director', {
                    defaultValue: '导演视角',
                  })
                : t('creativeStudio.director.workbench.viewMode.camera', {
                    defaultValue: '机位视角',
                  })}
            </Button>
          ))}
        </div>

        <div className={styles.topBarActions}>
          {props.headerActionsSlot}
          {props.onClose ? (
            <Tooltip
              content={t('creativeStudio.director.workbench.close', {
                defaultValue: '关闭导演台',
              })}
            >
              <Button
                shape='circle'
                type='secondary'
                aria-label={t('creativeStudio.director.workbench.close', {
                  defaultValue: '关闭导演台',
                })}
                icon={<Close />}
                onClick={props.onClose}
              />
            </Tooltip>
          ) : null}
        </div>
      </header>

      <div className={styles.workspace}>
        {!props.panelsCollapsed ? (
          <DirectorSceneSidebar
            sceneQuery={props.sceneQuery}
            sceneGroups={props.sceneGroups}
            disabled={props.disabled}
            onSceneQueryChange={props.onSceneQueryChange}
            onSceneObjectSelect={props.onSceneObjectSelect}
            onSceneObjectVisibilityChange={props.onSceneObjectVisibilityChange}
            onSceneObjectLockChange={props.onSceneObjectLockChange}
          />
        ) : null}

        <DirectorViewport
          viewportSlot={props.viewportSlot}
          viewportOverlaySlot={props.viewportOverlaySlot}
          gizmoSlot={props.gizmoSlot}
          transformMode={props.transformMode}
          modelLibraryOpen={props.modelLibraryOpen}
          modelLibraryItems={props.modelLibraryItems}
          aspectPickerOpen={props.aspectPickerOpen}
          aspectRatio={props.aspectRatio}
          showRuleOfThirds={props.showRuleOfThirds}
          panelsCollapsed={props.panelsCollapsed}
          timeline={props.timeline}
          disabled={props.disabled}
          captureBusy={props.captureBusy}
          onTransformModeChange={props.onTransformModeChange}
          onAddCharacter={props.onAddCharacter}
          onImportPanorama={props.onImportPanorama}
          onImportModel={props.onImportModel}
          onAddCamera={props.onAddCamera}
          onCaptureViewport={props.onCaptureViewport}
          onModelLibraryOpenChange={props.onModelLibraryOpenChange}
          onModelLibraryAdd={props.onModelLibraryAdd}
          onModelLibraryDelete={props.onModelLibraryDelete}
          onAspectPickerOpenChange={props.onAspectPickerOpenChange}
          onAspectRatioChange={props.onAspectRatioChange}
          onRuleOfThirdsChange={props.onRuleOfThirdsChange}
          onPanelsCollapsedChange={props.onPanelsCollapsedChange}
          onTimelineOpenChange={props.onTimelineOpenChange}
        />

        {!props.panelsCollapsed ? (
          <DirectorInspector
            inspector={props.inspector}
            bodyTypeOptions={props.bodyTypeOptions}
            posePresetOptions={props.posePresetOptions}
            disabled={props.disabled}
            captureBusy={props.captureBusy}
            onInspectorChange={props.onInspectorChange}
            onChoosePanorama={props.onChoosePanorama}
            onRemovePanorama={props.onRemovePanorama}
            onReimportObjectModel={props.onReimportObjectModel}
            onPosePresetSelect={props.onPosePresetSelect}
            onCameraCapture={props.onCameraCapture}
            onCaptureView={props.onCaptureView}
            onCaptureDelete={props.onCaptureDelete}
            onCaptureSendToCanvas={props.onCaptureSendToCanvas}
            onCaptureClearAll={props.onCaptureClearAll}
            onCaptureSendAll={props.onCaptureSendAll}
          />
        ) : null}

        <DirectorTimeline
          timeline={props.timeline}
          disabled={props.disabled}
          onTimelineOpenChange={props.onTimelineOpenChange}
          onTimelinePlayingChange={props.onTimelinePlayingChange}
          onTimelineLoopChange={props.onTimelineLoopChange}
          onTimelineAutoKeyChange={props.onTimelineAutoKeyChange}
          onTimelineTimeChange={props.onTimelineTimeChange}
          onTimelineDurationChange={props.onTimelineDurationChange}
          onTimelineTrackSelect={props.onTimelineTrackSelect}
          onKeyframeSelect={props.onKeyframeSelect}
          onKeyframeAdd={props.onKeyframeAdd}
          onKeyframeDelete={props.onKeyframeDelete}
          onTimelineExport={props.onTimelineExport}
        />
      </div>
    </section>
  );
};

export default DirectorWorkbenchShell;
