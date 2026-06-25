/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Event bus for the session secondary sidebar (ContentSider hosted by
 * ConversationShell). Mirrors the workspace-panel event bus so the Titlebar can
 * own a stable toggle button — present in the same place whether the panel is
 * open or closed — while ConversationShell owns the actual collapse state.
 *
 * - ConversationShell broadcasts STATE on mount and on every change.
 * - The Titlebar reflects STATE on its toggle icon and dispatches TOGGLE on click.
 */
export const SESSION_SIDER_TOGGLE_EVENT = 'nomifun-session-sider-toggle';
export const SESSION_SIDER_STATE_EVENT = 'nomifun-session-sider-state';

export interface SessionSiderStateDetail {
  collapsed: boolean;
}

export function dispatchSessionSiderToggleEvent() {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent(SESSION_SIDER_TOGGLE_EVENT));
}

export function dispatchSessionSiderStateEvent(collapsed: boolean) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(
    new CustomEvent<SessionSiderStateDetail>(SESSION_SIDER_STATE_EVENT, { detail: { collapsed } })
  );
}
