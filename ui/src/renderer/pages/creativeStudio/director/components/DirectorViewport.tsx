/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Add,
  Camera,
  Close,
  Cube,
  Delete,
  FullScreen,
  ImportAndExport,
  MoveOne,
  PeoplePlus,
  Picture,
  Rotate,
  Scale,
  Screenshot,
  Time,
  Upload,
  ViewGridCard,
} from '@icon-park/react';
import { Button, Tooltip } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import styles from './DirectorWorkbenchShell.module.css';
import {
  DIRECTOR_ASPECT_RATIO_OPTIONS,
  type DirectorTransformMode,
  type DirectorWorkbenchShellProps,
} from './types';

interface ToolbarActionProps {
  label: string;
  icon: React.ReactNode;
  active?: boolean;
  disabled?: boolean;
  onClick?(): void;
}

const ToolbarAction: React.FC<ToolbarActionProps> = ({
  label,
  icon,
  active = false,
  disabled = false,
  onClick,
}) => (
  <Tooltip content={label} position='top'>
    <Button
      type='text'
      shape='circle'
      className={styles.viewportToolbarButton}
      aria-label={label}
      aria-pressed={active}
      disabled={disabled || !onClick}
      icon={icon}
      onClick={onClick}
    />
  </Tooltip>
);

type DirectorViewportProps = Pick<
  DirectorWorkbenchShellProps,
  | 'viewportSlot'
  | 'viewportOverlaySlot'
  | 'gizmoSlot'
  | 'transformMode'
  | 'modelLibraryOpen'
  | 'modelLibraryItems'
  | 'aspectPickerOpen'
  | 'aspectRatio'
  | 'showRuleOfThirds'
  | 'panelsCollapsed'
  | 'timeline'
  | 'disabled'
  | 'captureBusy'
  | 'onTransformModeChange'
  | 'onAddCharacter'
  | 'onImportPanorama'
  | 'onImportModel'
  | 'onAddCamera'
  | 'onCaptureViewport'
  | 'onModelLibraryOpenChange'
  | 'onModelLibraryAdd'
  | 'onModelLibraryDelete'
  | 'onAspectPickerOpenChange'
  | 'onAspectRatioChange'
  | 'onRuleOfThirdsChange'
  | 'onPanelsCollapsedChange'
  | 'onTimelineOpenChange'
>;

const TransformButton: React.FC<{
  mode: DirectorTransformMode;
  currentMode: DirectorTransformMode;
  disabled: boolean;
  onChange(mode: DirectorTransformMode): void;
}> = ({ mode, currentMode, disabled, onChange }) => {
  const { t } = useTranslation();
  const config = {
    translate: {
      label: t('creativeStudio.director.viewport.transform.translate', {
        defaultValue: '移动',
      }),
      icon: <MoveOne />,
    },
    rotate: {
      label: t('creativeStudio.director.viewport.transform.rotate', {
        defaultValue: '旋转',
      }),
      icon: <Rotate />,
    },
    scale: {
      label: t('creativeStudio.director.viewport.transform.scale', {
        defaultValue: '缩放',
      }),
      icon: <Scale />,
    },
  }[mode];

  return (
    <ToolbarAction
      label={config.label}
      icon={config.icon}
      active={mode === currentMode}
      disabled={disabled}
      onClick={() => onChange(mode)}
    />
  );
};

const DirectorViewport: React.FC<DirectorViewportProps> = ({
  viewportSlot,
  viewportOverlaySlot,
  gizmoSlot,
  transformMode,
  modelLibraryOpen,
  modelLibraryItems,
  aspectPickerOpen,
  aspectRatio,
  showRuleOfThirds,
  panelsCollapsed,
  timeline,
  disabled = false,
  captureBusy = false,
  onTransformModeChange,
  onAddCharacter,
  onImportPanorama,
  onImportModel,
  onAddCamera,
  onCaptureViewport,
  onModelLibraryOpenChange,
  onModelLibraryAdd,
  onModelLibraryDelete,
  onAspectPickerOpenChange,
  onAspectRatioChange,
  onRuleOfThirdsChange,
  onPanelsCollapsedChange,
  onTimelineOpenChange,
}) => {
  const { t } = useTranslation();

  return (
  <main
    className={styles.viewport}
    aria-label={t('creativeStudio.director.viewport.title', {
      defaultValue: '3D视口',
    })}
    data-director-viewport
  >
    <div className={styles.viewportContent} data-director-viewport-slot>
      {viewportSlot}
    </div>

    {viewportOverlaySlot ? (
      <div className={styles.viewportOverlaySlot} data-director-viewport-overlay-slot>
        {viewportOverlaySlot}
      </div>
    ) : null}

    {aspectRatio !== 'free' || showRuleOfThirds ? (
      <div className={styles.aspectOverlay} aria-hidden='true' data-aspect-ratio={aspectRatio}>
        <div className={styles.aspectFrame}>
          {showRuleOfThirds ? (
            <div className={styles.ruleOfThirds} data-rule-of-thirds>
              <span className={styles.thirdVerticalOne} />
              <span className={styles.thirdVerticalTwo} />
              <span className={styles.thirdHorizontalOne} />
              <span className={styles.thirdHorizontalTwo} />
            </div>
          ) : null}
        </div>
      </div>
    ) : null}

    {gizmoSlot ? (
      <div className={styles.gizmoSlot} data-director-gizmo-slot>
        {gizmoSlot}
      </div>
    ) : null}

    <div
      className={styles.viewportToolbar}
      role='toolbar'
      aria-label={t('creativeStudio.director.viewport.toolbar', {
        defaultValue: '3D视口快捷工具',
      })}
      data-timeline-open={timeline.open}
    >
      {(['translate', 'rotate', 'scale'] as const).map((mode) => (
        <TransformButton
          key={mode}
          mode={mode}
          currentMode={transformMode}
          disabled={disabled}
          onChange={onTransformModeChange}
        />
      ))}
      <ToolbarAction
        label={t('creativeStudio.director.viewport.actions.addCharacter', {
          defaultValue: '添加角色',
        })}
        icon={<PeoplePlus />}
        disabled={disabled}
        onClick={onAddCharacter}
      />
      <ToolbarAction
        label={t('creativeStudio.director.viewport.actions.importPanorama', {
          defaultValue: '导入全景图',
        })}
        icon={<Picture />}
        disabled={disabled}
        onClick={onImportPanorama}
      />
      <ToolbarAction
        label={t('creativeStudio.director.viewport.actions.importModel', {
          defaultValue: '导入本地模型',
        })}
        icon={<Upload />}
        disabled={disabled}
        onClick={onImportModel}
      />
      <ToolbarAction
        label={t('creativeStudio.director.viewport.actions.modelLibrary', {
          defaultValue: '模型库',
        })}
        icon={<Cube />}
        active={modelLibraryOpen}
        disabled={disabled}
        onClick={() => onModelLibraryOpenChange(!modelLibraryOpen)}
      />
      <ToolbarAction
        label={t('creativeStudio.director.viewport.actions.addCamera', {
          defaultValue: '添加机位',
        })}
        icon={<Camera />}
        disabled={disabled}
        onClick={onAddCamera}
      />
      <ToolbarAction
        label={t('creativeStudio.director.viewport.actions.aspectRatio', {
          defaultValue: '选择画幅比例',
        })}
        icon={<ViewGridCard />}
        active={aspectPickerOpen}
        disabled={disabled}
        onClick={() => onAspectPickerOpenChange(!aspectPickerOpen)}
      />
      <ToolbarAction
        label={t('creativeStudio.director.viewport.actions.captureCurrent', {
          defaultValue: '当前视角截图',
        })}
        icon={<Screenshot />}
        disabled={disabled || captureBusy}
        onClick={onCaptureViewport ? () => onCaptureViewport('current') : undefined}
      />
      <ToolbarAction
        label={t('creativeStudio.director.viewport.actions.captureFour', {
          defaultValue: '四方位截图',
        })}
        icon={<ImportAndExport />}
        disabled={disabled || captureBusy}
        onClick={onCaptureViewport ? () => onCaptureViewport('four') : undefined}
      />
      <ToolbarAction
        label={t('creativeStudio.director.viewport.actions.captureTwelve', {
          defaultValue: '十二方位截图',
        })}
        icon={<Add />}
        disabled={disabled || captureBusy}
        onClick={onCaptureViewport ? () => onCaptureViewport('twelve') : undefined}
      />
      <ToolbarAction
        label={
          panelsCollapsed
            ? t('creativeStudio.director.viewport.actions.showSidebars', {
                defaultValue: '显示侧边栏',
              })
            : t('creativeStudio.director.viewport.actions.fullscreen', {
                defaultValue: '全屏视口',
              })
        }
        icon={<FullScreen />}
        active={panelsCollapsed}
        disabled={disabled}
        onClick={() => onPanelsCollapsedChange(!panelsCollapsed)}
      />
      <ToolbarAction
        label={t('creativeStudio.director.viewport.actions.timeline', {
          defaultValue: '时间轴',
        })}
        icon={<Time />}
        active={timeline.open}
        disabled={disabled}
        onClick={() => onTimelineOpenChange(!timeline.open)}
      />
    </div>

    {modelLibraryOpen ? (
      <section
        className={styles.modelLibrary}
        role='dialog'
        aria-label={t('creativeStudio.director.modelLibrary.title', {
          defaultValue: '模型库',
        })}
      >
        <header>
          <h2>
            {t('creativeStudio.director.modelLibrary.title', {
              defaultValue: '模型库',
            })}
          </h2>
          <Button
            type='text'
            shape='circle'
            size='small'
            aria-label={t('creativeStudio.director.modelLibrary.close', {
              defaultValue: '关闭模型库',
            })}
            icon={<Close />}
            onClick={() => onModelLibraryOpenChange(false)}
          />
        </header>
        <div
          className={styles.modelLibraryTabs}
          role='tablist'
          aria-label={t('creativeStudio.director.modelLibrary.categories', {
            defaultValue: '模型分类',
          })}
        >
          <button type='button' role='tab' aria-selected='true'>
            {t('creativeStudio.director.modelLibrary.myModels', {
              defaultValue: '我的模型',
            })}
          </button>
        </div>
        {modelLibraryItems.length === 0 ? (
          <div className={styles.modelLibraryEmpty} role='status'>
            <Cube aria-hidden='true' size={24} strokeWidth={1.7} />
            <span>
              {t('creativeStudio.director.modelLibrary.empty', {
                defaultValue: '暂无任何模型',
              })}
            </span>
            <Button disabled={disabled || !onImportModel} onClick={onImportModel}>
              {t('creativeStudio.director.modelLibrary.importLocal', {
                defaultValue: '本地导入',
              })}
            </Button>
          </div>
        ) : (
          <div
            className={styles.modelLibraryGrid}
            role='list'
            aria-label={t('creativeStudio.director.modelLibrary.list', {
              defaultValue: '模型列表',
            })}
          >
            {modelLibraryItems.map((model) => (
              <article key={model.id} className={styles.modelCard}>
                <button
                  type='button'
                  aria-label={t('creativeStudio.director.modelLibrary.addModel', {
                    defaultValue: '添加模型 {{name}}',
                    name: model.name,
                  })}
                  disabled={disabled || !onModelLibraryAdd}
                  onClick={() => onModelLibraryAdd?.(model.id)}
                >
                  <span className={styles.modelThumb} aria-hidden='true'>
                    {model.thumbnailUrl ? (
                      <img src={model.thumbnailUrl} alt='' />
                    ) : (
                      <Cube size={24} strokeWidth={1.7} />
                    )}
                  </span>
                  <span title={model.name}>{model.name}</span>
                </button>
                {model.deletable ? (
                  <Button
                    type='text'
                    shape='circle'
                    size='mini'
                    className={styles.modelDelete}
                    aria-label={t('creativeStudio.director.modelLibrary.deleteModel', {
                      defaultValue: '删除模型 {{name}}',
                      name: model.name,
                    })}
                    icon={<Delete />}
                    disabled={disabled || !onModelLibraryDelete}
                    onClick={() => onModelLibraryDelete?.(model.id)}
                  />
                ) : null}
              </article>
            ))}
            <button
              type='button'
              className={styles.modelImportCard}
              disabled={disabled || !onImportModel}
              onClick={onImportModel}
            >
              <span className={styles.modelThumb} aria-hidden='true'>
                <Upload size={24} strokeWidth={1.7} />
              </span>
              <span>
                {t('creativeStudio.director.modelLibrary.importLocal', {
                  defaultValue: '本地导入',
                })}
              </span>
            </button>
          </div>
        )}
      </section>
    ) : null}

    {aspectPickerOpen ? (
      <section
        className={styles.aspectPicker}
        role='dialog'
        aria-label={t('creativeStudio.director.aspect.title', {
          defaultValue: '比例',
        })}
      >
        <header>
          <h2>
            {t('creativeStudio.director.aspect.title', {
              defaultValue: '比例',
            })}
          </h2>
          <CheckboxProxy
            checked={showRuleOfThirds}
            disabled={disabled}
            onChange={onRuleOfThirdsChange}
          />
        </header>
        <div
          className={styles.aspectOptions}
          role='group'
          aria-label={t('creativeStudio.director.aspect.options', {
            defaultValue: '画幅比例选项',
          })}
        >
          {DIRECTOR_ASPECT_RATIO_OPTIONS.map((option) => (
            <Button
              key={option}
              type={aspectRatio === option ? 'primary' : 'secondary'}
              aria-pressed={aspectRatio === option}
              disabled={disabled}
              onClick={() => onAspectRatioChange(option)}
            >
              {option === 'free'
                ? t('creativeStudio.director.aspect.free', {
                    defaultValue: '自由',
                  })
                : option}
            </Button>
          ))}
        </div>
      </section>
    ) : null}
  </main>
  );
};

const CheckboxProxy: React.FC<{
  checked: boolean;
  disabled: boolean;
  onChange(value: boolean): void;
}> = ({ checked, disabled, onChange }) => {
  const { t } = useTranslation();
  return (
    <label className={styles.thirdsToggle}>
      <input
        type='checkbox'
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
      {t('creativeStudio.director.aspect.ruleOfThirds', {
        defaultValue: '三分线',
      })}
    </label>
  );
};

export default DirectorViewport;
