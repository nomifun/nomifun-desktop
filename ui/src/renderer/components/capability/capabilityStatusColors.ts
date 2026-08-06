/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AutoWorkRunState, IdmmRunState, ISshLinkPhase } from '@/common/adapter/ipcBridge';

import { CAPABILITY_COLORS } from './CapabilityIcon';

/**
 * Per-capability run-state → colour, derived from the shared {@link CAPABILITY_COLORS}
 * palette. This is the SINGLE routing table both surfaces read:
 *  - the conversation-header controls (AutoWorkControl / IdmmControl) colour their
 *    trigger icon + status marker through it, and
 *  - the session-list capability icons (sessionCapabilityItems) colour the row icon
 *    through it.
 * Keeping the state→colour mapping here (not re-inlined per surface) is what keeps
 * the header and the sidebar from drifting — the bug that had IDMM `off` resolve to
 * gray in the header but blue in the sidebar.
 */
export const AUTOWORK_STATUS_COLOR: Record<AutoWorkRunState, string> = {
  off: CAPABILITY_COLORS.off,
  idle: CAPABILITY_COLORS.idle,
  active: CAPABILITY_COLORS.active,
};

export const IDMM_STATUS_COLOR: Record<IdmmRunState, string> = {
  off: CAPABILITY_COLORS.off,
  armed: CAPABILITY_COLORS.armed,
  intervening: CAPABILITY_COLORS.active,
};

/**
 * SSH link phase → colour for the conversation-header host pill.
 *
 * The phase alone decides the hue. `detail` is free-form operator text and must
 * never be string-matched for a colour — that is exactly how a "connection
 * refused" message ends up rendering green.
 *
 * - `connected` is the only green: commands can actually reach the host.
 * - `dropped` is the only red: the link is gone and the session is stuck.
 * - `degraded` (transport alive, remote shell being recycled) and
 *   `reconnecting` (backoff ladder running) are both amber-armed: work is
 *   temporarily not flowing, but nothing is broken for good.
 * - `connecting` is the neutral in-flight tint; `idle` and `closed` are off —
 *   there is no link, which is not a fault. A `closed` link whose exit could not
 *   be proven still says so in the popover (`reaped === false`), because that is
 *   a warning about the *remote host*, not about this link's colour.
 */
export const SSH_STATUS_COLOR: Record<ISshLinkPhase, string> = {
  idle: CAPABILITY_COLORS.off,
  connecting: CAPABILITY_COLORS.idle,
  connected: CAPABILITY_COLORS.active,
  degraded: CAPABILITY_COLORS.armed,
  reconnecting: CAPABILITY_COLORS.armed,
  dropped: CAPABILITY_COLORS.danger,
  closed: CAPABILITY_COLORS.off,
};
