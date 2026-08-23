/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  ArrowLeft,
  BookOpen,
  CheckOne,
  Close,
  Dot,
  Error,
  FolderOpen,
  GridFour,
  Group,
  HandDrag,
  History,
  Loading,
  Magic,
  MenuFold,
  PanoramaHorizontal,
  Pic,
  Platte,
  Redo,
  Robot,
  Setting,
  SettingConfig,
  Square,
  Text,
  Timeline,
  Undo,
  VideoTwo,
  Voice,
  Workbench,
} from '@icon-park/react';
import { Tooltip } from '@arco-design/web-react';
import classNames from 'classnames';
import React, {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';

import styles from './CreativeCanvasChrome.module.css';
import {
  CREATIVE_CANVAS_CHROME_BACKGROUNDS,
  CREATIVE_CANVAS_CHROME_NODE_KINDS,
  CREATIVE_CANVAS_CHROME_TOOLBAR_NODE_KINDS,
  toggleCreativeCanvasBottomPanel,
  toggleCreativeCanvasPanel,
  toggleCreativeCanvasTool,
  type CreativeCanvasBottomView,
  type CreativeCanvasChromeBackground,
  type CreativeCanvasChromeNodeKind,
  type CreativeCanvasChromeProps,
  type CreativeCanvasChromeSaveStatus,
  type CreativeCanvasLeftView,
  type CreativeCanvasRightView,
} from './types';

const NODE_LABEL_KEYS: Record<CreativeCanvasChromeNodeKind, string> = {
  text: 'creativeStudio.canvas.nodeKinds.text',
  image: 'creativeStudio.canvas.nodeKinds.image',
  panorama: 'creativeStudio.canvas.nodeKinds.panorama',
  video: 'creativeStudio.canvas.nodeKinds.video',
  audio: 'creativeStudio.canvas.nodeKinds.audio',
  config: 'creativeStudio.canvas.nodeKinds.config',
  director: 'creativeStudio.canvas.nodeKinds.director',
  group: 'creativeStudio.canvas.nodeKinds.group',
};

const BACKGROUND_LABEL_KEYS: Record<CreativeCanvasChromeBackground, string> = {
  dots: 'creativeStudio.canvas.backgrounds.dots',
  lines: 'creativeStudio.canvas.backgrounds.lines',
  blank: 'creativeStudio.canvas.backgrounds.blank',
};

const LEFT_LABEL_KEYS: Record<CreativeCanvasLeftView, string> = {
  canvas: 'creativeStudio.canvas.panels.left.canvas',
  assets: 'creativeStudio.canvas.panels.left.assets',
  prompts: 'creativeStudio.canvas.panels.left.prompts',
  templates: 'creativeStudio.canvas.panels.left.templates',
};

const RIGHT_LABEL_KEYS: Record<CreativeCanvasRightView, string> = {
  assistant: 'creativeStudio.canvas.panels.right.assistant',
  properties: 'creativeStudio.canvas.panels.right.properties',
};

const BOTTOM_LABEL_KEYS: Record<CreativeCanvasBottomView, string> = {
  history: 'creativeStudio.canvas.panels.bottom.history',
  timeline: 'creativeStudio.canvas.panels.bottom.timeline',
};

const SAVE_LABEL_KEYS: Record<CreativeCanvasChromeSaveStatus, string> = {
  idle: 'creativeStudio.canvas.save.status.idle',
  dirty: 'creativeStudio.canvas.save.status.dirty',
  saving: 'creativeStudio.canvas.save.status.saving',
  saved: 'creativeStudio.canvas.save.status.saved',
  conflict: 'creativeStudio.canvas.save.status.conflict',
  error: 'creativeStudio.canvas.save.status.error',
};

const iconProps = {
  theme: 'outline' as const,
  size: 17,
  fill: 'currentColor',
  strokeWidth: 3,
};

const RIGHT_PANEL_DEFAULT_WIDTH = 390;
const RIGHT_PANEL_MIN_WIDTH = 320;
const RIGHT_PANEL_MAX_WIDTH = 560;
const RIGHT_PANEL_COMPACT_MIN_WIDTH = 260;
const MIN_CANVAS_STAGE_WIDTH = 360;

interface RightPanelWidthBounds {
  min: number;
  max: number;
}

const rightPanelWidthBounds = (
  containerWidth: number
): RightPanelWidthBounds => {
  const width =
    Number.isFinite(containerWidth) && containerWidth > 0
      ? containerWidth
      : 1200;
  const min =
    width <= 640
      ? 240
      : width < 760
        ? RIGHT_PANEL_COMPACT_MIN_WIDTH
        : RIGHT_PANEL_MIN_WIDTH;
  const max = Math.max(
    min,
    Math.min(RIGHT_PANEL_MAX_WIDTH, width - MIN_CANVAS_STAGE_WIDTH)
  );
  return { min, max };
};

const clampRightPanelWidth = (
  value: number,
  bounds: RightPanelWidthBounds
): number =>
  Math.round(
    Math.max(
      bounds.min,
      Math.min(
        bounds.max,
        Number.isFinite(value) ? value : RIGHT_PANEL_DEFAULT_WIDTH
      )
    )
  );

function nodeIcon(kind: CreativeCanvasChromeNodeKind): React.ReactNode {
  switch (kind) {
    case 'text':
      return <Text {...iconProps} />;
    case 'image':
      return <Pic {...iconProps} />;
    case 'panorama':
      return <PanoramaHorizontal {...iconProps} />;
    case 'video':
      return <VideoTwo {...iconProps} />;
    case 'audio':
      return <Voice {...iconProps} />;
    case 'config':
      return <SettingConfig {...iconProps} />;
    case 'director':
      return <Magic {...iconProps} />;
    case 'group':
      return <Group {...iconProps} />;
  }
}

function backgroundIcon(background: CreativeCanvasChromeBackground): React.ReactNode {
  if (background === 'dots') return <Dot {...iconProps} />;
  if (background === 'lines') return <GridFour {...iconProps} />;
  return <Square {...iconProps} />;
}

function leftIcon(view: CreativeCanvasLeftView): React.ReactNode {
  if (view === 'canvas') return <Platte {...iconProps} />;
  if (view === 'assets') return <FolderOpen {...iconProps} />;
  if (view === 'prompts') return <BookOpen {...iconProps} />;
  return <Workbench {...iconProps} />;
}

function rightIcon(view: CreativeCanvasRightView): React.ReactNode {
  return view === 'assistant' ? <Robot {...iconProps} /> : <Setting {...iconProps} />;
}

function bottomIcon(view: CreativeCanvasBottomView): React.ReactNode {
  return view === 'history' ? <History {...iconProps} /> : <Timeline {...iconProps} />;
}

function saveIcon(status: CreativeCanvasChromeSaveStatus): React.ReactNode {
  if (status === 'saving') return <Loading className={styles.spin} {...iconProps} />;
  if (status === 'saved') return <CheckOne {...iconProps} />;
  if (status === 'conflict' || status === 'error') return <Error {...iconProps} />;
  return <Dot {...iconProps} />;
}

interface ChromeIconButtonProps {
  label: string;
  icon: React.ReactNode;
  active?: boolean;
  disabled?: boolean;
  danger?: boolean;
  onClick(): void;
}

const ChromeIconButton: React.FC<ChromeIconButtonProps> = ({
  label,
  icon,
  active,
  disabled,
  danger,
  onClick,
}) => (
  <Tooltip content={label} position='top' mini>
    <button
      type='button'
      className={styles.iconButton}
      data-active={active || undefined}
      data-danger={danger || undefined}
      aria-label={label}
      aria-pressed={active}
      disabled={disabled}
      onClick={onClick}
    >
      {icon}
    </button>
  </Tooltip>
);

export interface CreativeCanvasNodeMenuProps {
  disabled?: boolean;
  onSelect(kind: CreativeCanvasChromeNodeKind): void;
}

export const CreativeCanvasNodeMenu: React.FC<CreativeCanvasNodeMenuProps> = ({
  disabled,
  onSelect,
}) => {
  const { t } = useTranslation();
  const label = t('creativeStudio.canvas.chrome.addNode');

  return (
    <div className={styles.nodeMenu} role='menu' aria-label={label} data-canvas-node-menu>
      <div className={styles.menuHeading}>{label}</div>
      <div className={styles.nodeMenuGrid}>
        {CREATIVE_CANVAS_CHROME_NODE_KINDS.map((kind) => (
          <button
            key={kind}
            type='button'
            role='menuitem'
            data-node-kind={kind}
            disabled={disabled}
            onClick={() => onSelect(kind)}
          >
            <span className={styles.nodeMenuIcon}>{nodeIcon(kind)}</span>
            <span>{t(NODE_LABEL_KEYS[kind])}</span>
          </button>
        ))}
      </div>
    </div>
  );
};

export interface CreativeCanvasBackgroundMenuProps {
  value: CreativeCanvasChromeBackground;
  disabled?: boolean;
  onChange(background: CreativeCanvasChromeBackground): void;
}

export const CreativeCanvasBackgroundMenu: React.FC<CreativeCanvasBackgroundMenuProps> = ({
  value,
  disabled,
  onChange,
}) => {
  const { t } = useTranslation();
  const label = t('creativeStudio.canvas.chrome.canvasBackground');

  return (
    <div
      className={styles.backgroundMenu}
      role='menu'
      aria-label={label}
      data-canvas-background-menu
    >
      <div className={styles.menuHeading}>{label}</div>
      {CREATIVE_CANVAS_CHROME_BACKGROUNDS.map((background) => (
        <button
          key={background}
          type='button'
          role='menuitemradio'
          aria-checked={background === value}
          data-background={background}
          data-active={background === value || undefined}
          disabled={disabled}
          onClick={() => onChange(background)}
        >
          {backgroundIcon(background)}
          <span>{t(BACKGROUND_LABEL_KEYS[background])}</span>
          {background === value ? <CheckOne {...iconProps} /> : null}
        </button>
      ))}
    </div>
  );
};

const stopChromeEvent = (event: React.SyntheticEvent) => event.stopPropagation();

const CreativeCanvasChrome: React.FC<CreativeCanvasChromeProps> = (props) => {
  const { t } = useTranslation();
  const rightOpen = props.rightView !== null;
  const saveIsAlert = props.saveStatus === 'conflict' || props.saveStatus === 'error';
  const rootRef = useRef<HTMLElement>(null);
  const resizeCleanupRef = useRef<(() => void) | null>(null);
  const [containerWidth, setContainerWidth] = useState(1200);
  const [isRightPanelResizing, setIsRightPanelResizing] = useState(false);
  const [rightPanelWidthDraft, setRightPanelWidthDraft] = useState(
    props.rightPanelWidth ?? RIGHT_PANEL_DEFAULT_WIDTH
  );
  const leftSlot = props.slots?.left?.[props.leftView];
  const rightSlot = props.rightView ? props.slots?.right?.[props.rightView] : null;
  const bottomSlot = props.bottomView ? props.slots?.bottom?.[props.bottomView] : null;
  const widthBounds = rightPanelWidthBounds(containerWidth);
  const rightPanelWidth = clampRightPanelWidth(
    rightPanelWidthDraft,
    widthBounds
  );
  const chromeEventProps = {
    onPointerDown: stopChromeEvent,
    onPointerMove: stopChromeEvent,
    onPointerUp: stopChromeEvent,
    onDoubleClick: stopChromeEvent,
    onWheel: stopChromeEvent,
  };

  const collapseResourcesLabel = t(
    'creativeStudio.canvas.chrome.collapseResources'
  );
  const resizeRightPanelLabel = t(
    'creativeStudio.canvas.chrome.resizeRightPanel'
  );
  const resetRightPanelWidthLabel = t(
    'creativeStudio.canvas.chrome.resetRightPanelWidth'
  );

  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const update = () => {
      const width = root.getBoundingClientRect().width;
      if (width > 0) setContainerWidth(width);
    };
    update();
    if (typeof ResizeObserver !== 'undefined') {
      const observer = new ResizeObserver(update);
      observer.observe(root);
      return () => observer.disconnect();
    }
    window.addEventListener('resize', update);
    return () => window.removeEventListener('resize', update);
  }, []);

  useLayoutEffect(() => {
    if (!isRightPanelResizing) {
      setRightPanelWidthDraft(
        props.rightPanelWidth ?? RIGHT_PANEL_DEFAULT_WIDTH
      );
    }
  }, [isRightPanelResizing, props.rightPanelWidth]);

  useLayoutEffect(
    () => () => {
      resizeCleanupRef.current?.();
      resizeCleanupRef.current = null;
    },
    []
  );

  const commitRightPanelWidth = useCallback(
    (width: number) => {
      const nextWidth = clampRightPanelWidth(
        width,
        rightPanelWidthBounds(
          rootRef.current?.getBoundingClientRect().width ?? containerWidth
        )
      );
      setRightPanelWidthDraft(nextWidth);
      props.onRightPanelWidthChange?.(nextWidth);
    },
    [containerWidth, props.onRightPanelWidthChange]
  );

  const handleRightPanelResizeStart = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (
        props.disabled ||
        !rightOpen ||
        !props.onRightPanelWidthChange ||
        (event.pointerType !== 'touch' && event.button !== 0)
      ) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      resizeCleanupRef.current?.();

      const handle = event.currentTarget;
      const startX = event.clientX;
      const startWidth = rightPanelWidth;
      const bounds = rightPanelWidthBounds(
        rootRef.current?.getBoundingClientRect().width ?? containerWidth
      );
      const pointerId = event.pointerId;
      let dragging = true;
      let latestWidth = startWidth;
      const previousUserSelect = document.body.style.userSelect;
      const previousCursor = document.body.style.cursor;

      const updateWidth = (clientX: number) => {
        latestWidth = clampRightPanelWidth(
          startWidth - (clientX - startX),
          bounds
        );
        setRightPanelWidthDraft(latestWidth);
      };
      const cleanup = () => {
        document.body.style.userSelect = previousUserSelect;
        document.body.style.cursor = previousCursor;
        handle.removeEventListener(
          'lostpointercapture',
          handleLostPointerCapture
        );
        if (
          handle.releasePointerCapture &&
          handle.hasPointerCapture?.(pointerId)
        ) {
          handle.releasePointerCapture(pointerId);
        }
        window.removeEventListener('pointermove', handlePointerMove, true);
        window.removeEventListener('pointerup', handlePointerUp, true);
        window.removeEventListener('pointercancel', handlePointerCancel, true);
        window.removeEventListener('blur', handleBlur, true);
        resizeCleanupRef.current = null;
      };
      const finish = (clientX?: number) => {
        if (!dragging) return;
        dragging = false;
        if (typeof clientX === 'number') updateWidth(clientX);
        cleanup();
        setIsRightPanelResizing(false);
        props.onRightPanelWidthChange?.(latestWidth);
      };
      const handlePointerMove = (moveEvent: PointerEvent) => {
        if (!dragging) return;
        if (moveEvent.buttons === 0) {
          finish(moveEvent.clientX);
          return;
        }
        updateWidth(moveEvent.clientX);
      };
      const handlePointerUp = (upEvent: PointerEvent) => finish(upEvent.clientX);
      const handlePointerCancel = () => finish();
      const handleBlur = () => finish();
      const handleLostPointerCapture = () => finish();

      document.body.style.userSelect = 'none';
      document.body.style.cursor = 'col-resize';
      setIsRightPanelResizing(true);
      try {
        handle.setPointerCapture(pointerId);
        handle.addEventListener('lostpointercapture', handleLostPointerCapture);
      } catch {
        // Window listeners still complete the drag if capture is unavailable.
      }
      window.addEventListener('pointermove', handlePointerMove, true);
      window.addEventListener('pointerup', handlePointerUp, true);
      window.addEventListener('pointercancel', handlePointerCancel, true);
      window.addEventListener('blur', handleBlur, true);
      resizeCleanupRef.current = cleanup;
    },
    [
      containerWidth,
      props.disabled,
      props.onRightPanelWidthChange,
      rightOpen,
      rightPanelWidth,
    ]
  );

  const handleRightPanelResizeKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (props.disabled || !props.onRightPanelWidthChange) return;
      const step = event.shiftKey ? 32 : 16;
      if (event.key === 'ArrowLeft') {
        event.preventDefault();
        commitRightPanelWidth(rightPanelWidth + step);
      } else if (event.key === 'ArrowRight') {
        event.preventDefault();
        commitRightPanelWidth(rightPanelWidth - step);
      } else if (event.key === 'Home') {
        event.preventDefault();
        commitRightPanelWidth(widthBounds.max);
      } else if (event.key === 'End') {
        event.preventDefault();
        commitRightPanelWidth(widthBounds.min);
      }
    },
    [
      commitRightPanelWidth,
      props.disabled,
      props.onRightPanelWidthChange,
      rightPanelWidth,
      widthBounds.max,
      widthBounds.min,
    ]
  );

  return (
    <section
      ref={rootRef}
      className={classNames(styles.root, props.className)}
      style={
        props.rightPanelWidth !== undefined
          ? ({
              '--creative-canvas-right-panel-width': `${rightPanelWidth}px`,
            } as React.CSSProperties)
          : undefined
      }
      data-creative-canvas-chrome
      data-compact={props.compact || undefined}
      data-left-open={props.leftOpen}
      data-left-view={props.leftView}
      data-right-view={props.rightView ?? 'closed'}
      data-bottom-view={props.bottomView ?? 'closed'}
      aria-label={t('creativeStudio.canvas.chrome.workspaceControls')}
    >
      <header className={styles.topBar} data-canvas-no-zoom {...chromeEventProps}>
        <button
          type='button'
          className={styles.backButton}
          disabled={props.disabled}
          onClick={props.onBackToCanvases}
        >
          <ArrowLeft {...iconProps} />
          <span className={styles.backLabel}>
            {t('creativeStudio.canvas.chrome.backToLibrary')}
          </span>
        </button>

        <div className={styles.projectIdentity}>
          <h1 title={props.canvasTitle}>{props.canvasTitle}</h1>
          <div
            className={styles.saveState}
            data-save-status={props.saveStatus}
            role={saveIsAlert ? 'alert' : 'status'}
            aria-live='polite'
          >
            {saveIcon(props.saveStatus)}
            <span>{props.saveMessage ?? t(SAVE_LABEL_KEYS[props.saveStatus])}</span>
          </div>
        </div>

        <div className={styles.topActions}>
          {props.slots?.topActions ? (
            <div className={styles.customTopActions}>{props.slots.topActions}</div>
          ) : null}
          {(['assistant', 'properties'] as const).map((view) => (
            <ChromeIconButton
              key={view}
              label={t(RIGHT_LABEL_KEYS[view])}
              icon={rightIcon(view)}
              active={props.rightView === view}
              disabled={props.disabled}
              onClick={() =>
                props.onRightViewChange(toggleCreativeCanvasPanel(props.rightView, view))
              }
            />
          ))}
        </div>
      </header>

      <aside
        className={styles.leftPanel}
        aria-label={t('creativeStudio.canvas.chrome.resourcePanel')}
        data-left-open={props.leftOpen}
        data-canvas-no-zoom
        {...chromeEventProps}
      >
        <nav
          className={styles.leftTabs}
          aria-label={t('creativeStudio.canvas.chrome.resources')}
          role='tablist'
        >
          {(['canvas', 'assets', 'prompts', 'templates'] as const).map((view) => {
            const label = t(LEFT_LABEL_KEYS[view]);
            return (
              <Tooltip key={view} content={label} position='right' mini>
                <button
                  type='button'
                  role='tab'
                  id={`creative-canvas-left-tab-${view}`}
                  aria-label={label}
                  aria-selected={props.leftOpen && props.leftView === view}
                  aria-expanded={props.leftOpen && props.leftView === view}
                  aria-controls='creative-canvas-left-panel-body'
                  data-active={
                    props.leftOpen && props.leftView === view
                      ? true
                      : undefined
                  }
                  disabled={props.disabled}
                  title={label}
                  onClick={() => {
                    if (props.leftOpen && props.leftView === view) {
                      props.onLeftPanelOpenChange(false);
                      return;
                    }
                    props.onLeftViewChange(view);
                  }}
                >
                  {leftIcon(view)}
                  <span className={styles.tabLabel}>{label}</span>
                </button>
              </Tooltip>
            );
          })}
          {props.leftOpen ? (
            <Tooltip content={collapseResourcesLabel} position='right' mini>
              <button
                type='button'
                className={styles.leftCollapseButton}
                aria-label={collapseResourcesLabel}
                title={collapseResourcesLabel}
                disabled={props.disabled}
                onClick={() => props.onLeftPanelOpenChange(false)}
              >
                <MenuFold {...iconProps} />
              </button>
            </Tooltip>
          ) : null}
        </nav>
        <div
          className={styles.panelBody}
          id='creative-canvas-left-panel-body'
          role='tabpanel'
          aria-labelledby={`creative-canvas-left-tab-${props.leftView}`}
          aria-hidden={!props.leftOpen}
          hidden={!props.leftOpen}
          data-left-panel-body={props.leftView}
        >
          {leftSlot}
        </div>
      </aside>

      <div className={styles.canvasStage} data-canvas-chrome-stage>
        {props.slots?.canvas}
      </div>

      {rightOpen && props.rightView ? (
        <aside
          className={styles.rightPanel}
          aria-label={t(RIGHT_LABEL_KEYS[props.rightView])}
          data-resizing={isRightPanelResizing || undefined}
          data-canvas-no-zoom
          {...chromeEventProps}
        >
          {props.onRightPanelWidthChange ? (
            <div
              className={styles.rightResizeHandle}
              role='separator'
              tabIndex={props.disabled ? -1 : 0}
              aria-label={resizeRightPanelLabel}
              aria-orientation='vertical'
              aria-valuemin={widthBounds.min}
              aria-valuemax={widthBounds.max}
              aria-valuenow={rightPanelWidth}
              aria-valuetext={`${rightPanelWidth}px`}
              aria-disabled={props.disabled || undefined}
              title={resizeRightPanelLabel}
              onPointerDown={handleRightPanelResizeStart}
              onKeyDown={handleRightPanelResizeKeyDown}
              onDoubleClick={() =>
                commitRightPanelWidth(RIGHT_PANEL_DEFAULT_WIDTH)
              }
            >
              <span
                className={styles.rightResizeLine}
                aria-hidden='true'
                title={resetRightPanelWidthLabel}
              />
            </div>
          ) : null}
          {props.rightView === 'properties' ? (
            <header className={styles.panelHeader} data-right-panel-header='properties'>
              <div
                className={styles.panelTabs}
                role='tablist'
                aria-label={t('creativeStudio.canvas.chrome.rightPanel')}
              >
                {(['assistant', 'properties'] as const).map((view) => (
                  <button
                    key={view}
                    type='button'
                    role='tab'
                    aria-selected={props.rightView === view}
                    data-active={props.rightView === view || undefined}
                    disabled={props.disabled}
                    onClick={() => props.onRightViewChange(view)}
                  >
                    {rightIcon(view)}
                    <span>{t(RIGHT_LABEL_KEYS[view])}</span>
                  </button>
                ))}
              </div>
              <ChromeIconButton
                label={t('creativeStudio.canvas.chrome.closeRightPanel')}
                icon={<Close {...iconProps} />}
                disabled={props.disabled}
                onClick={() => props.onRightViewChange(null)}
              />
            </header>
          ) : null}
          <div className={styles.panelBody} role='tabpanel' data-right-panel-body={props.rightView}>
            {rightSlot}
          </div>
        </aside>
      ) : null}

      {props.bottomView ? (
        <section
          className={styles.bottomPanel}
          aria-label={t(BOTTOM_LABEL_KEYS[props.bottomView])}
          data-canvas-no-zoom
          {...chromeEventProps}
        >
          <header className={styles.panelHeader}>
            <div
              className={styles.panelTabs}
              role='tablist'
              aria-label={t('creativeStudio.canvas.chrome.bottomPanel')}
            >
              {(['history', 'timeline'] as const).map((view) => (
                <button
                  key={view}
                  type='button'
                  role='tab'
                  aria-selected={props.bottomView === view}
                  data-active={props.bottomView === view || undefined}
                  disabled={props.disabled}
                  onClick={() => props.onBottomViewChange(view)}
                >
                  {bottomIcon(view)}
                  <span>{t(BOTTOM_LABEL_KEYS[view])}</span>
                </button>
              ))}
            </div>
            <ChromeIconButton
              label={t('creativeStudio.canvas.chrome.closeBottomPanel')}
              icon={<Close {...iconProps} />}
              disabled={props.disabled}
              onClick={() => props.onBottomViewChange(null)}
            />
          </header>
          <div
            className={styles.panelBody}
            role='tabpanel'
            data-bottom-panel-body={props.bottomView}
          >
            {bottomSlot}
          </div>
        </section>
      ) : null}

      <div className={styles.toolbarPositioner}>
        <div
          className={styles.toolDock}
          role='toolbar'
          aria-label={t('creativeStudio.canvas.chrome.toolbar')}
          data-canvas-no-zoom
          {...chromeEventProps}
        >
          <ChromeIconButton
            label={t('creativeStudio.canvas.actions.panTool')}
            icon={<HandDrag {...iconProps} />}
            active={props.tool === 'pan'}
            disabled={props.disabled}
            onClick={() => props.onToolChange(toggleCreativeCanvasTool(props.tool))}
          />

          <span className={styles.divider} aria-hidden='true' />

          <ChromeIconButton
            label={t('creativeStudio.canvas.actions.undo')}
            icon={<Undo {...iconProps} />}
            disabled={props.disabled || !props.canUndo}
            onClick={props.onUndo}
          />
          <ChromeIconButton
            label={t('creativeStudio.canvas.actions.redo')}
            icon={<Redo {...iconProps} />}
            disabled={props.disabled || !props.canRedo}
            onClick={props.onRedo}
          />

          <span className={styles.divider} aria-hidden='true' />

          {CREATIVE_CANVAS_CHROME_TOOLBAR_NODE_KINDS.map((kind) => (
            <ChromeIconButton
              key={kind}
              label={t(NODE_LABEL_KEYS[kind])}
              icon={nodeIcon(kind)}
              disabled={props.disabled}
              onClick={() => props.onAddNode(kind)}
            />
          ))}

          <span className={styles.divider} aria-hidden='true' />

          <ChromeIconButton
            label={t(BOTTOM_LABEL_KEYS.history)}
            icon={<History {...iconProps} />}
            active={props.bottomView !== null}
            disabled={props.disabled}
            onClick={() =>
              props.onBottomViewChange(toggleCreativeCanvasBottomPanel(props.bottomView))
            }
          />
          {props.slots?.toolbarTrailing}
        </div>
      </div>
    </section>
  );
};

export default CreativeCanvasChrome;
