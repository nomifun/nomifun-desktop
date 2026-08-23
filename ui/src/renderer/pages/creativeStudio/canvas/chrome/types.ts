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

export type CreativeCanvasLeftView = 'canvas' | 'assets' | 'prompts' | 'templates';
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
  /** @deprecated Background selection now lives in the zoom popover. */
  background?: CreativeCanvasChromeBackground;
  canUndo: boolean;
  canRedo: boolean;
  leftOpen: boolean;
  leftView: CreativeCanvasLeftView;
  rightView: CreativeCanvasRightView | null;
  /** Current persisted width of the right panel, in CSS pixels. */
  rightPanelWidth?: number;
  bottomView: CreativeCanvasBottomView | null;
  /** @deprecated Background selection now lives in the zoom popover. */
  backgroundMenuOpen?: boolean;
  compact?: boolean;
  disabled?: boolean;
  className?: string;
  slots?: CreativeCanvasChromeSlots;
  onBackToCanvases(): void;
  onToolChange(tool: CreativeCanvasChromeTool): void;
  onAddNode(kind: CreativeCanvasChromeNodeKind): void;
  /** @deprecated Background selection now lives in the zoom popover. */
  onBackgroundChange?(background: CreativeCanvasChromeBackground): void;
  /** @deprecated Background selection now lives in the zoom popover. */
  onBackgroundMenuOpenChange?(open: boolean): void;
  onUndo(): void;
  onRedo(): void;
  onLeftPanelOpenChange(open: boolean): void;
  onLeftViewChange(view: CreativeCanvasLeftView): void;
  onRightViewChange(view: CreativeCanvasRightView | null): void;
  /** Persist a user-adjusted right panel width, in CSS pixels. */
  onRightPanelWidthChange?(width: number): void;
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

export function toggleCreativeCanvasTool(
  current: CreativeCanvasChromeTool
): CreativeCanvasChromeTool {
  return current === 'pan' ? 'select' : 'pan';
}

export function toggleCreativeCanvasBottomPanel(
  current: CreativeCanvasBottomView | null
): CreativeCanvasBottomView | null {
  return current === null ? 'history' : null;
}
