/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { NormalizedToolCall } from '@/common/chat/normalizeToolCall';
import type { TurnDisclosureProcessState } from '../turnDisclosureModel';

export interface ToolSummaryDescriptor {
  target: string;
  count: number;
}

const stateMatchesTool = (state: TurnDisclosureProcessState, tool: NormalizedToolCall): boolean => {
  if (state === 'running') return tool.status === 'running' || tool.status === 'pending';
  if (state === 'failed') return tool.status === 'error';
  if (state === 'canceled') return tool.status === 'canceled';
  if (state === 'completed') return tool.status === 'completed';
  return tool.status === 'pending' || tool.status === 'running';
};

const formatToolTarget = (tool: NormalizedToolCall): string => {
  const name = tool.name?.trim();
  const description = tool.description?.trim();
  if (name && description && description !== name) return `${name} ${description}`;
  return name || description || tool.key;
};

export const buildToolSummaryDescriptor = (
  tools: NormalizedToolCall[],
  state: TurnDisclosureProcessState
): ToolSummaryDescriptor | null => {
  if (!tools.length) return null;

  const focusedTool = tools.findLast((tool) => stateMatchesTool(state, tool)) ?? tools.at(-1);
  if (!focusedTool) return null;

  return {
    target: formatToolTarget(focusedTool),
    count: tools.length,
  };
};
