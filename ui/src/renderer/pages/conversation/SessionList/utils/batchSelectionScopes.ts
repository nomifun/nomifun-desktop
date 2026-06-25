/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { SessionKind, WorkpathNode } from './workpathTree';

export type BatchSelectableScope = {
  conversationIds: number[];
  terminalIds: number[];
};

export type BatchSelectionState = {
  conversationIds: Set<number>;
  terminalIds: Set<number>;
};

export type BatchSelectionScopeState = {
  checked: boolean;
  indeterminate: boolean;
  disabled: boolean;
  selectedCount: number;
  totalCount: number;
};

const idsForKind = (node: WorkpathNode, kind?: SessionKind) => ({
  conversationIds: kind === 'terminal' ? [] : node.interactive.map((entry) => entry.id),
  terminalIds: kind === 'interactive' ? [] : node.terminal.map((entry) => entry.id),
});

const hasAny = (scope: BatchSelectableScope) =>
  scope.conversationIds.length > 0 || scope.terminalIds.length > 0;

export function getWorkpathBatchSelectionScope(node: WorkpathNode, kind?: SessionKind): BatchSelectableScope {
  return idsForKind(node, kind);
}

export function isBatchSelectionScopeFullySelected(
  scope: BatchSelectableScope,
  selected: BatchSelectionState
) {
  return (
    hasAny(scope) &&
    scope.conversationIds.every((id) => selected.conversationIds.has(id)) &&
    scope.terminalIds.every((id) => selected.terminalIds.has(id))
  );
}

export function getBatchSelectionScopeState(
  scope: BatchSelectableScope,
  selected: BatchSelectionState
): BatchSelectionScopeState {
  const totalCount = scope.conversationIds.length + scope.terminalIds.length;
  const selectedCount =
    scope.conversationIds.filter((id) => selected.conversationIds.has(id)).length +
    scope.terminalIds.filter((id) => selected.terminalIds.has(id)).length;

  if (totalCount === 0 || selectedCount === 0) {
    return {
      checked: false,
      indeterminate: false,
      disabled: totalCount === 0,
      selectedCount,
      totalCount,
    };
  }

  return {
    checked: true,
    indeterminate: selectedCount < totalCount,
    disabled: false,
    selectedCount,
    totalCount,
  };
}

export function toggleBatchSelectionScope(
  scope: BatchSelectableScope,
  selected: BatchSelectionState
): BatchSelectionState {
  const nextConversationIds = new Set(selected.conversationIds);
  const nextTerminalIds = new Set(selected.terminalIds);
  const remove = isBatchSelectionScopeFullySelected(scope, selected);

  for (const id of scope.conversationIds) {
    if (remove) nextConversationIds.delete(id);
    else nextConversationIds.add(id);
  }
  for (const id of scope.terminalIds) {
    if (remove) nextTerminalIds.delete(id);
    else nextTerminalIds.add(id);
  }

  return {
    conversationIds: nextConversationIds,
    terminalIds: nextTerminalIds,
  };
}
