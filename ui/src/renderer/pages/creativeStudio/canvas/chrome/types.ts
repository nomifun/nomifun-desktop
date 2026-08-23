/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ReactNode } from 'react';

import type { CreativeCanvasBackground, CreativeCanvasNode } from '../../domain';
import type { CanvasInteractionTool } from '../components';
import type { CanvasCasSaveStatus } from '../editor';

export type CreativeCanvasChromeNodeKind = CreativeCanvasNode['type'];
export type CreativeCanvasChromeBackground = CreativeCanvasBackground;
export type CreativeCanvasChromeTool = CanvasInteractionTool;
export type CreativeCanvasChromeSaveStatus = CanvasCasSaveStatus;

export type CreativeCanvasLeftView = 'canvas' | 'assets' | 'prompts' | 'workflows';
export type CreativeCanvasRightView = 'assistant' | 'properties';
export type CreativeCanvasBottomView = 'history' | 'timeline';

export interface CreativeCanvasChromeSlots {
  canvas?: ReactNode;
  topActions?: ReactNode;
  toolbarTrailing?: ReactNode;
  left?: Partial<Record<CreativeCanvasLeftView, ReactNode>>;
  right?: Partial<Record<CreativeCanvasRightView, ReactNode>>;
  bottom?: Partial<Record<CreativeCanvasBottomView, ReactNode>>;
}

export interface CreativeCanvasChromeProps {
  canvasTitle: string;
  saveStatus: CreativeCanvasChromeSaveStatus;
  saveMessage?: string;
  tool: CreativeCanvasChromeTool;
  background: CreativeCanvasChromeBackground;
  canUndo: boolean;
  canRedo: boolean;
  isMiniMapOpen: boolean;
  leftOpen: boolean;
  leftView: CreativeCanvasLeftView;
  rightView: CreativeCanvasRightView | null;
  bottomView: CreativeCanvasBottomView | null;
  backgroundMenuOpen: boolean;
  compact?: boolean;
  disabled?: boolean;
  className?: string;
  slots?: CreativeCanvasChromeSlots;
  onBackToCanvases(): void;
  onToolChange(tool: CreativeCanvasChromeTool): void;
  onAddNode(kind: CreativeCanvasChromeNodeKind): void;
  onBackgroundChange(background: CreativeCanvasChromeBackground): void;
  onBackgroundMenuOpenChange(open: boolean): void;
  onUndo(): void;
  onRedo(): void;
  onFitView(): void;
  onToggleMiniMap(): void;
  onLeftPanelOpenChange(open: boolean): void;
  onLeftViewChange(view: CreativeCanvasLeftView): void;
  onRightViewChange(view: CreativeCanvasRightView | null): void;
  onBottomViewChange(view: CreativeCanvasBottomView | null): void;
}

export const CREATIVE_CANVAS_CHROME_NODE_KINDS = [
  'text',
  'image',
  'panorama',
  'video',
  'audio',
  'config',
  'director',
  'group',
] as const satisfies readonly CreativeCanvasChromeNodeKind[];

export const CREATIVE_CANVAS_CHROME_TOOLBAR_NODE_KINDS = [
  'text',
  'image',
  'video',
  'audio',
  'panorama',
  'director',
  'config',
] as const satisfies readonly CreativeCanvasChromeNodeKind[];

export const CREATIVE_CANVAS_CHROME_BACKGROUNDS = [
  'dots',
  'lines',
  'blank',
] as const satisfies readonly CreativeCanvasChromeBackground[];

export function toggleCreativeCanvasPanel<T extends string>(current: T | null, target: T): T | null {
  return current === target ? null : target;
}
