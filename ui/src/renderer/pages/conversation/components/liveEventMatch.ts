/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * IDs are canonical strings at both protocol and component boundaries. Strict
 * equality intentionally rejects numeric compatibility representations.
 */
export const isLiveEventForTarget = (
  eventKind: string,
  eventTargetId: string,
  kind: string,
  id: string,
): boolean => eventKind === kind && eventTargetId === id;
