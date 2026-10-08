/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ReactNode } from 'react';

import type {
  CreativeCanvasUserNodeKind,
} from '../../domain';
import type { CanvasInteractionTool } from '../components';
import type { CanvasCasSaveStatus } from '../editor';

export type CreativeCanvasChromeNodeKind = CreativeCanvasUserNodeKind;
export type CreativeCanvasChromeTool = CanvasInteractionTool;
export type CreativeCanvasChromeSaveStatus = CanvasCasSaveStatus;

export type CreativeCanvasLeftView = 'canvas' | 'assets' | 'prompts' | 'templates';
export type CreativeCanvasResourceView = Exclude<CreativeCanvasLeftView, 'canvas'>;
export type CreativeCanvasRightView = 'assistant' | 'properties';

export interface CreativeCanvasChromeSlots {
  canvas?: ReactNode;
  topActions?: ReactNode;
  toolbarTrailing?: ReactNode;
  left?: Partial<Record<CreativeCanvasLeftView, ReactNode>>;
  right?: Partial<Record<CreativeCanvasRightView, ReactNode>>;
}

export interface CreativeCanvasChromeProps {
  canvasId?: string;
  canvasTitle: string;
  onRenameCanvas?(title: string): Promise<void>;
  saveStatus: CreativeCanvasChromeSaveStatus;
  saveMessage?: string;
  tool: CreativeCanvasChromeTool;
  canUndo: boolean;
  canRedo: boolean;
  leftOpen: boolean;
  leftView: CreativeCanvasLeftView;
  /** Resource libraries use one shared modal instead of occupying the canvas rail. */
  resourceView: CreativeCanvasResourceView | null;
  /** Mount the resource modal inside this element while the Canvas owns fullscreen. */
  resourceDialogPopupContainer?: HTMLElement | null;
  rightView: CreativeCanvasRightView | null;
  /** Current persisted width of the right panel, in CSS pixels. */
  rightPanelWidth?: number;
  compact?: boolean;
  disabled?: boolean;
  className?: string;
  slots?: CreativeCanvasChromeSlots;
  onBackToCanvases(): void;
  onToolChange(tool: CreativeCanvasChromeTool): void;
  onAddNode(kind: CreativeCanvasChromeNodeKind): void;
  onUndo(): void;
  onRedo(): void;
  onLeftPanelOpenChange(open: boolean): void;
  onLeftViewChange(view: CreativeCanvasLeftView): void;
  onResourceViewChange(view: CreativeCanvasResourceView | null): void;
  onRightViewChange(view: CreativeCanvasRightView | null): void;
  /** Persist a user-adjusted right panel width, in CSS pixels. */
  onRightPanelWidthChange?(width: number): void;
}

export const CREATIVE_CANVAS_CHROME_NODE_KINDS = [
  'text',
  'image',
  'video',
  'audio',
  'timeline',
  'group',
] as const satisfies readonly CreativeCanvasChromeNodeKind[];

export const CREATIVE_CANVAS_CHROME_TOOLBAR_NODE_KINDS = [
  'text',
  'image',
  'video',
  'audio',
  'timeline',
] as const satisfies readonly CreativeCanvasChromeNodeKind[];

export function toggleCreativeCanvasPanel<T extends string>(current: T | null, target: T): T | null {
  return current === target ? null : target;
}

export function toggleCreativeCanvasTool(
  current: CreativeCanvasChromeTool
): CreativeCanvasChromeTool {
  return current === 'pan' ? 'select' : 'pan';
}
