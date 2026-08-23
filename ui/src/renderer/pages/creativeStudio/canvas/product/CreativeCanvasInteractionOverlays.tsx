/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';

import { CreativeCanvasNodeMenu } from '../chrome';
import type { CreativeCanvasNodeKind, CreativeSize } from '../../domain';
import type {
  CanvasContextAction,
  CanvasContextTarget,
} from '../interactions';
import type { CanvasPoint } from '../core';
import styles from './CreativeCanvasInteractionOverlays.module.css';

export interface CreativeCanvasContextMenuState {
  target: CanvasContextTarget;
  clientPosition: CanvasPoint;
  nodeLocked?: boolean;
}

export interface CreativeCanvasCreateNodeMenuState {
  clientPosition: CanvasPoint;
}

export interface CreativeCanvasInteractionOverlaysProps {
  viewportSize: CreativeSize;
  contextMenu: CreativeCanvasContextMenuState | null;
  createNodeMenu: CreativeCanvasCreateNodeMenuState | null;
  disabled?: boolean;
  onContextAction(action: CanvasContextAction): void;
  onOpenCreateNodeMenu(): void;
  onPasteFromSystemClipboard(): void;
  onSelectNode(kind: CreativeCanvasNodeKind): void;
  onDismiss(): void;
}

const clampPosition = (
  point: CanvasPoint,
  viewport: CreativeSize,
  menu: CreativeSize
): React.CSSProperties => ({
  left: Math.min(Math.max(8, point.x), Math.max(8, viewport.width - menu.width - 8)),
  top: Math.min(Math.max(8, point.y), Math.max(8, viewport.height - menu.height - 8)),
});

const MenuButton: React.FC<{
  children: React.ReactNode;
  danger?: boolean;
  onClick(): void;
}> = ({ children, danger, onClick }) => (
  <button type='button' role='menuitem' data-danger={danger || undefined} onClick={onClick}>
    {children}
  </button>
);

/** Screen-space menus for typed canvas intents; no document state is owned here. */
const CreativeCanvasInteractionOverlays: React.FC<CreativeCanvasInteractionOverlaysProps> = ({
  viewportSize,
  contextMenu,
  createNodeMenu,
  disabled,
  onContextAction,
  onOpenCreateNodeMenu,
  onPasteFromSystemClipboard,
  onSelectNode,
  onDismiss,
}) => {
  const { t } = useTranslation();
  if (!contextMenu && !createNodeMenu) return null;

  return (
    <div
      className={styles.dismissLayer}
      data-canvas-interaction-overlay
      onPointerDown={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onDismiss();
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
    >
      {contextMenu ? (
        <div
          className={styles.contextMenu}
          style={clampPosition(contextMenu.clientPosition, viewportSize, {
            width: 188,
            height: contextMenu.target.kind === 'node' ? 196 : 108,
          })}
          role='menu'
          aria-label={t('creativeStudio.canvas.contextMenu.label', {
            defaultValue: '画布上下文菜单',
          })}
          onPointerDown={(event) => event.stopPropagation()}
        >
          {contextMenu.target.kind === 'canvas' ? (
            <>
              <MenuButton onClick={onOpenCreateNodeMenu}>
                {t('creativeStudio.canvas.contextMenu.addNode', {
                  defaultValue: '添加节点',
                })}
              </MenuButton>
              <MenuButton onClick={onPasteFromSystemClipboard}>
                {t('creativeStudio.canvas.contextMenu.pasteClipboard', {
                  defaultValue: '从系统剪贴板粘贴',
                })}
              </MenuButton>
            </>
          ) : null}
          {contextMenu.target.kind === 'node' ? (
            <>
              <MenuButton onClick={() => onContextAction('open')}>
                {t('creativeStudio.canvas.contextMenu.open', {
                  defaultValue: '打开',
                })}
              </MenuButton>
              <MenuButton onClick={() => onContextAction('duplicate')}>
                {t('creativeStudio.canvas.contextMenu.duplicate', {
                  defaultValue: '创建副本',
                })}
              </MenuButton>
              <MenuButton onClick={() => onContextAction('toggle-lock')}>
                {contextMenu.nodeLocked
                  ? t('creativeStudio.canvas.contextMenu.unlockNode', {
                      defaultValue: '解锁节点',
                    })
                  : t('creativeStudio.canvas.contextMenu.lockNode', {
                      defaultValue: '锁定节点',
                    })}
              </MenuButton>
              <MenuButton danger onClick={() => onContextAction('delete')}>
                {t('creativeStudio.canvas.contextMenu.deleteNode', {
                  defaultValue: '删除节点',
                })}
              </MenuButton>
            </>
          ) : null}
          {contextMenu.target.kind === 'edge' ? (
            <MenuButton danger onClick={() => onContextAction('delete')}>
              {t('creativeStudio.canvas.contextMenu.deleteConnection', {
                defaultValue: '删除连接',
              })}
            </MenuButton>
          ) : null}
        </div>
      ) : null}

      {createNodeMenu ? (
        <div
          className={styles.nodeMenuSurface}
          style={clampPosition(createNodeMenu.clientPosition, viewportSize, {
            width: 288,
            height: 226,
          })}
          onPointerDown={(event) => event.stopPropagation()}
        >
          <CreativeCanvasNodeMenu disabled={disabled} onSelect={onSelectNode} />
        </div>
      ) : null}
    </div>
  );
};

export default CreativeCanvasInteractionOverlays;
