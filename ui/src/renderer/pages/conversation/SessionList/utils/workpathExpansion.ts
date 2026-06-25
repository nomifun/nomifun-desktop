/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type RenderedExpansionStateOptions = {
  active: boolean;
  persistedExpanded: boolean;
  activeRouteSynced?: boolean;
};

export type RenderedExpansionState = {
  expanded: boolean;
  shouldSyncExpanded: boolean;
};

/**
 * Active routes should reveal their owning drawer once, but they must not keep
 * overriding a user's explicit collapse click while that route stays open.
 */
export function getRenderedExpansionState({
  active,
  persistedExpanded,
  activeRouteSynced = false,
}: RenderedExpansionStateOptions): RenderedExpansionState {
  return {
    expanded: persistedExpanded,
    shouldSyncExpanded: active && !persistedExpanded && !activeRouteSynced,
  };
}
