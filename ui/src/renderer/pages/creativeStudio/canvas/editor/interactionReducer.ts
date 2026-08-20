/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CanvasPoint } from '../core';

export type CanvasPointerGesture =
  | {
      kind: 'pan';
      pointerId: number;
      lastClient: CanvasPoint;
    }
  | {
      kind: 'select';
      pointerId: number;
      lastClient: CanvasPoint;
    }
  | {
      kind: 'move';
      pointerId: number;
      lastClient: CanvasPoint;
      mergeKey: string;
    };

export interface CanvasEditorInteractionState {
  gesture: CanvasPointerGesture | null;
  isPanning: boolean;
}

export type CanvasEditorInteractionAction =
  | { type: 'gesture/start'; gesture: CanvasPointerGesture }
  | { type: 'gesture/update'; pointerId: number; client: CanvasPoint }
  | { type: 'gesture/end'; pointerId?: number };

export const INITIAL_CANVAS_EDITOR_INTERACTION: CanvasEditorInteractionState = {
  gesture: null,
  isPanning: false,
};

export function canvasEditorInteractionReducer(
  state: CanvasEditorInteractionState,
  action: CanvasEditorInteractionAction
): CanvasEditorInteractionState {
  switch (action.type) {
    case 'gesture/start':
      return {
        gesture: action.gesture,
        isPanning: action.gesture.kind === 'pan',
      };
    case 'gesture/update':
      if (!state.gesture || state.gesture.pointerId !== action.pointerId) return state;
      return {
        ...state,
        gesture: { ...state.gesture, lastClient: { ...action.client } },
      };
    case 'gesture/end':
      if (
        action.pointerId !== undefined &&
        state.gesture &&
        state.gesture.pointerId !== action.pointerId
      ) {
        return state;
      }
      return INITIAL_CANVAS_EDITOR_INTERACTION;
  }
}
