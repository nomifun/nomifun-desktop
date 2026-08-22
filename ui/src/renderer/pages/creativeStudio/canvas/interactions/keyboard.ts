/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { canvasCommands, type CanvasPoint, type CanvasState } from '../core';
import {
  canvasInteractionResolution,
  type CanvasInteractionResolution,
  unhandledCanvasInteraction,
} from './types';

/** A DOM-independent projection of the fields used from KeyboardEvent. */
export interface CanvasKeyboardInput {
  key: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
  altKey?: boolean;
  /** The renderer decides this with its existing editable-target guard. */
  editable?: boolean;
}

export interface CanvasKeyboardOptions {
  /** One-based repeated-paste index, owned by the editor session. */
  pasteSequence?: number;
  /** Optional placement for a real system-clipboard fallback. */
  clipboardWorldPosition?: CanvasPoint;
  at?: number;
}

const hasSelection = (state: CanvasState): boolean =>
  state.selection.nodeIds.length > 0 || state.selection.edgeIds.length > 0;

/**
 * Resolve source-compatible canvas shortcuts without touching window,
 * navigator.clipboard, storage, or an asset API.
 */
export function resolveCanvasKeyboardInput(
  state: CanvasState,
  input: CanvasKeyboardInput,
  options: CanvasKeyboardOptions = {}
): CanvasInteractionResolution {
  if (input.editable) return unhandledCanvasInteraction();

  const key = input.key.toLocaleLowerCase();
  const modifier = Boolean(input.ctrlKey || input.metaKey);
  const commandModifier = modifier && !input.altKey;

  if (commandModifier && key === 'z') {
    return canvasInteractionResolution({
      commands: [input.shiftKey ? canvasCommands.redo() : canvasCommands.undo()],
    });
  }
  if (commandModifier && key === 'y') {
    return canvasInteractionResolution({ commands: [canvasCommands.redo()] });
  }
  if (commandModifier && key === 'a') {
    return canvasInteractionResolution({
      commands: [
        canvasCommands.setSelection(
          state.document.nodes.map((node) => node.id),
          []
        ),
      ],
    });
  }
  if (commandModifier && key === 'g') {
    return canvasInteractionResolution({
      commands: [canvasCommands.groupNodes({ at: options.at })],
    });
  }
  if (commandModifier && key === 'c') {
    return canvasInteractionResolution({
      commands: [canvasCommands.copySelection()],
    });
  }
  if (commandModifier && key === 'v') {
    const pasteSequence = Math.max(1, Math.trunc(options.pasteSequence ?? 1));
    const paste = canvasCommands.pasteClipboard(state, {
      offset: { x: 32 * pasteSequence, y: 32 * pasteSequence },
      at: options.at,
    });
    return canvasInteractionResolution(
      paste
        ? { commands: [paste] }
        : {
            intents: [
              {
                type: 'system-clipboard/read',
                ...(options.clipboardWorldPosition
                  ? { worldPosition: { ...options.clipboardWorldPosition } }
                  : {}),
              },
            ],
          }
    );
  }
  if (input.key === 'Delete' || input.key === 'Backspace') {
    if (!hasSelection(state)) return unhandledCanvasInteraction();
    return canvasInteractionResolution({
      commands: [canvasCommands.deleteSelection({ at: options.at })],
    });
  }
  if (input.key === 'Escape') {
    return canvasInteractionResolution({
      preventDefault: false,
      commands: [canvasCommands.clearSelection()],
      intents: [{ type: 'transient-ui/dismiss' }],
    });
  }

  return unhandledCanvasInteraction();
}
