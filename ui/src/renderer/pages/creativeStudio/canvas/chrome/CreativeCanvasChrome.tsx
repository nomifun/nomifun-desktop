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
  Compass,
  Dot,
  Error,
  FolderOpen,
  FullScreen,
  GridFour,
  Group,
  HandDrag,
  History,
  Loading,
  Magic,
  Mouse,
  PanoramaHorizontal,
  Pic,
  Platte,
  Plus,
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
import { Popover, Tooltip } from '@arco-design/web-react';
import classNames from 'classnames';
import React from 'react';

import styles from './CreativeCanvasChrome.module.css';
import {
  CREATIVE_CANVAS_CHROME_BACKGROUNDS,
  CREATIVE_CANVAS_CHROME_NODE_KINDS,
  toggleCreativeCanvasPanel,
  type CreativeCanvasBottomView,
  type CreativeCanvasChromeBackground,
  type CreativeCanvasChromeNodeKind,
  type CreativeCanvasChromeProps,
  type CreativeCanvasChromeSaveStatus,
  type CreativeCanvasLeftView,
  type CreativeCanvasRightView,
} from './types';

const NODE_LABELS: Record<CreativeCanvasChromeNodeKind, string> = {
  text: '文本',
  image: '图片',
  panorama: '全景图',
  video: '视频',
  audio: '音频',
  config: '生成配置',
  director: '导演台',
  group: '分组',
};

const BACKGROUND_LABELS: Record<CreativeCanvasChromeBackground, string> = {
  dots: '点阵',
  lines: '网格线',
  blank: '空白',
};

const LEFT_LABELS: Record<CreativeCanvasLeftView, string> = {
  canvas: '画布',
  assets: '资产',
  prompts: '提示词',
  workflows: '工作流',
};

const RIGHT_LABELS: Record<CreativeCanvasRightView, string> = {
  assistant: '创作助手',
  properties: '属性',
};

const BOTTOM_LABELS: Record<CreativeCanvasBottomView, string> = {
  history: '历史',
  timeline: '时间线',
};

const SAVE_LABELS: Record<CreativeCanvasChromeSaveStatus, string> = {
  idle: '等待编辑',
  dirty: '有未保存更改',
  saving: '正在保存',
  saved: '已保存',
  conflict: '保存冲突',
  error: '保存失败',
};

const iconProps = {
  theme: 'outline' as const,
  size: 17,
  fill: 'currentColor',
  strokeWidth: 2.5,
};

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
}) => (
  <div className={styles.nodeMenu} role='menu' aria-label='添加节点' data-canvas-node-menu>
    <div className={styles.menuHeading}>添加节点</div>
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
          <span>{NODE_LABELS[kind]}</span>
        </button>
      ))}
    </div>
  </div>
);

export interface CreativeCanvasBackgroundMenuProps {
  value: CreativeCanvasChromeBackground;
  disabled?: boolean;
  onChange(background: CreativeCanvasChromeBackground): void;
}

export const CreativeCanvasBackgroundMenu: React.FC<CreativeCanvasBackgroundMenuProps> = ({
  value,
  disabled,
  onChange,
}) => (
  <div
    className={styles.backgroundMenu}
    role='menu'
    aria-label='画布背景'
    data-canvas-background-menu
  >
    <div className={styles.menuHeading}>画布背景</div>
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
        <span>{BACKGROUND_LABELS[background]}</span>
        {background === value ? <CheckOne {...iconProps} /> : null}
      </button>
    ))}
  </div>
);

const stopChromeEvent = (event: React.SyntheticEvent) => event.stopPropagation();

const CreativeCanvasChrome: React.FC<CreativeCanvasChromeProps> = (props) => {
  const rightOpen = props.rightView !== null;
  const saveIsAlert = props.saveStatus === 'conflict' || props.saveStatus === 'error';
  const leftSlot = props.slots?.left?.[props.leftView];
  const rightSlot = props.rightView ? props.slots?.right?.[props.rightView] : null;
  const bottomSlot = props.bottomView ? props.slots?.bottom?.[props.bottomView] : null;
  const chromeEventProps = {
    onPointerDown: stopChromeEvent,
    onPointerMove: stopChromeEvent,
    onPointerUp: stopChromeEvent,
    onDoubleClick: stopChromeEvent,
    onWheel: stopChromeEvent,
  };

  const selectNode = (kind: CreativeCanvasChromeNodeKind) => {
    props.onAddNode(kind);
    props.onNodeMenuOpenChange(false);
  };

  const selectBackground = (background: CreativeCanvasChromeBackground) => {
    props.onBackgroundChange(background);
    props.onBackgroundMenuOpenChange(false);
  };

  return (
    <section
      className={classNames(styles.root, props.className)}
      data-creative-canvas-chrome
      data-compact={props.compact || undefined}
      data-left-view={props.leftView}
      data-right-view={props.rightView ?? 'closed'}
      data-bottom-view={props.bottomView ?? 'closed'}
      aria-label='画布工作区控件'
    >
      <header className={styles.topBar} data-canvas-no-zoom {...chromeEventProps}>
        <button
          type='button'
          className={styles.backButton}
          disabled={props.disabled}
          onClick={props.onBackToProjects}
        >
          <ArrowLeft {...iconProps} />
          <span className={styles.backLabel}>返回项目</span>
        </button>

        <div className={styles.projectIdentity}>
          <h1 title={props.projectTitle}>{props.projectTitle}</h1>
          <div
            className={styles.saveState}
            data-save-status={props.saveStatus}
            role={saveIsAlert ? 'alert' : 'status'}
            aria-live='polite'
          >
            {saveIcon(props.saveStatus)}
            <span>{props.saveMessage ?? SAVE_LABELS[props.saveStatus]}</span>
          </div>
        </div>

        <div className={styles.topActions}>
          {props.slots?.topActions ? (
            <div className={styles.customTopActions}>{props.slots.topActions}</div>
          ) : null}
          {(['assistant', 'properties'] as const).map((view) => (
            <ChromeIconButton
              key={view}
              label={RIGHT_LABELS[view]}
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
        aria-label='画布资源面板'
        data-canvas-no-zoom
        {...chromeEventProps}
      >
        <nav className={styles.leftTabs} aria-label='画布资源' role='tablist'>
          {(['canvas', 'assets', 'prompts', 'workflows'] as const).map((view) => (
            <button
              key={view}
              type='button'
              role='tab'
              aria-selected={props.leftView === view}
              data-active={props.leftView === view || undefined}
              disabled={props.disabled}
              onClick={() => props.onLeftViewChange(view)}
            >
              {leftIcon(view)}
              <span className={styles.tabLabel}>{LEFT_LABELS[view]}</span>
            </button>
          ))}
        </nav>
        <div className={styles.panelBody} role='tabpanel' data-left-panel-body={props.leftView}>
          {leftSlot}
        </div>
      </aside>

      <div className={styles.canvasStage} data-canvas-chrome-stage>
        {props.slots?.canvas}
      </div>

      {rightOpen && props.rightView ? (
        <aside
          className={styles.rightPanel}
          aria-label={RIGHT_LABELS[props.rightView]}
          data-canvas-no-zoom
          {...chromeEventProps}
        >
          <header className={styles.panelHeader}>
            <div className={styles.panelTabs} role='tablist' aria-label='右侧面板'>
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
                  <span>{RIGHT_LABELS[view]}</span>
                </button>
              ))}
            </div>
            <ChromeIconButton
              label='关闭右侧面板'
              icon={<Close {...iconProps} />}
              disabled={props.disabled}
              onClick={() => props.onRightViewChange(null)}
            />
          </header>
          <div className={styles.panelBody} role='tabpanel' data-right-panel-body={props.rightView}>
            {rightSlot}
          </div>
        </aside>
      ) : null}

      {props.bottomView ? (
        <section
          className={styles.bottomPanel}
          aria-label={BOTTOM_LABELS[props.bottomView]}
          data-canvas-no-zoom
          {...chromeEventProps}
        >
          <header className={styles.panelHeader}>
            <div className={styles.panelTabs} role='tablist' aria-label='底部面板'>
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
                  <span>{BOTTOM_LABELS[view]}</span>
                </button>
              ))}
            </div>
            <ChromeIconButton
              label='关闭底部面板'
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
          aria-label='画布工具栏'
          data-canvas-no-zoom
          {...chromeEventProps}
        >
          <div className={styles.toolGroup}>
            <ChromeIconButton
              label='选择工具'
              icon={<Mouse {...iconProps} />}
              active={props.tool === 'select'}
              disabled={props.disabled}
              onClick={() => props.onToolChange('select')}
            />
            <ChromeIconButton
              label='平移工具'
              icon={<HandDrag {...iconProps} />}
              active={props.tool === 'pan'}
              disabled={props.disabled}
              onClick={() => props.onToolChange('pan')}
            />
          </div>

          <span className={styles.divider} aria-hidden='true' />

          <Popover
            trigger='click'
            position='top'
            popupVisible={props.nodeMenuOpen}
            onVisibleChange={props.onNodeMenuOpenChange}
            content={<CreativeCanvasNodeMenu disabled={props.disabled} onSelect={selectNode} />}
            unmountOnExit
          >
            <span className={styles.menuAnchor}>
              <button
                type='button'
                className={styles.addButton}
                data-active={props.nodeMenuOpen || undefined}
                aria-haspopup='menu'
                aria-expanded={props.nodeMenuOpen}
                disabled={props.disabled}
                onClick={() => props.onNodeMenuOpenChange(!props.nodeMenuOpen)}
              >
                <Plus {...iconProps} />
                <span className={styles.toolText}>添加节点</span>
              </button>
            </span>
          </Popover>

          <span className={styles.divider} aria-hidden='true' />

          <ChromeIconButton
            label='撤销'
            icon={<Undo {...iconProps} />}
            disabled={props.disabled || !props.canUndo}
            onClick={props.onUndo}
          />
          <ChromeIconButton
            label='重做'
            icon={<Redo {...iconProps} />}
            disabled={props.disabled || !props.canRedo}
            onClick={props.onRedo}
          />

          <span className={styles.divider} aria-hidden='true' />

          <Popover
            trigger='click'
            position='top'
            popupVisible={props.backgroundMenuOpen}
            onVisibleChange={props.onBackgroundMenuOpenChange}
            content={
              <CreativeCanvasBackgroundMenu
                value={props.background}
                disabled={props.disabled}
                onChange={selectBackground}
              />
            }
            unmountOnExit
          >
            <span className={styles.menuAnchor}>
              <ChromeIconButton
                label={`画布背景：${BACKGROUND_LABELS[props.background]}`}
                icon={backgroundIcon(props.background)}
                active={props.backgroundMenuOpen}
                disabled={props.disabled}
                onClick={() => props.onBackgroundMenuOpenChange(!props.backgroundMenuOpen)}
              />
            </span>
          </Popover>

          <ChromeIconButton
            label='适应内容'
            icon={<FullScreen {...iconProps} />}
            disabled={props.disabled}
            onClick={props.onFitView}
          />
          <ChromeIconButton
            label={props.isMiniMapOpen ? '关闭小地图' : '打开小地图'}
            icon={<Compass {...iconProps} />}
            active={props.isMiniMapOpen}
            disabled={props.disabled}
            onClick={props.onToggleMiniMap}
          />

          <span className={styles.divider} aria-hidden='true' />

          {(['history', 'timeline'] as const).map((view) => (
            <ChromeIconButton
              key={view}
              label={BOTTOM_LABELS[view]}
              icon={bottomIcon(view)}
              active={props.bottomView === view}
              disabled={props.disabled}
              onClick={() =>
                props.onBottomViewChange(toggleCreativeCanvasPanel(props.bottomView, view))
              }
            />
          ))}
          {props.slots?.toolbarTrailing}
        </div>
      </div>
    </section>
  );
};

export default CreativeCanvasChrome;
