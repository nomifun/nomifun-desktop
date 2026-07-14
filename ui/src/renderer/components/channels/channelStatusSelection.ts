/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import type { ChannelPlatform } from '@/renderer/components/settings/SettingsModal/contents/channels/channelTarget';

export interface EnabledChannelStatusQuery {
  platform: ChannelPlatform;
  enabledPluginId?: string;
  companionId?: string;
  publicAgentId?: string;
}

export type ChannelConfigTarget = { platform: ChannelPlatform; channelId?: string } | null;

export interface ChannelOwnerQuery {
  companionId?: string;
  publicAgentId?: string;
}

const nonEmpty = (value: string | null | undefined) => value?.trim() || undefined;

export function findEnabledChannelStatus(
  statuses: IChannelPluginStatus[],
  query: EnabledChannelStatusQuery
): IChannelPluginStatus | null {
  const enabledPluginId = nonEmpty(query.enabledPluginId);
  if (enabledPluginId) {
    const byId = statuses.find((status) => status.id === enabledPluginId);
    if (byId) return byId;
  }

  const companionId = nonEmpty(query.companionId);
  const publicAgentId = nonEmpty(query.publicAgentId);
  return (
    statuses.find((status) => {
      if (status.type !== query.platform) return false;
      if (publicAgentId) return nonEmpty(status.publicAgentId) === publicAgentId;
      if (companionId) return nonEmpty(status.companionId) === companionId;
      return false;
    }) ?? null
  );
}

/**
 * When the config modal is in create mode (no channelId), move it onto the row
 * the caller just resolved. The caller — findEnabledChannelStatus (by the
 * backend-returned channel id) or the owner-scoped adopt effect — already
 * guarantees `status` is the intended row, so we retarget by ROW ID rather than
 * re-checking owner equality, which was fragile against id normalization /
 * binding-commit-lag skew and left the toggle stuck OFF after a real success.
 */
export function retargetConfigAfterStatus(
  current: ChannelConfigTarget,
  status: IChannelPluginStatus | null
): ChannelConfigTarget {
  if (!current || current.channelId || !status || status.type !== current.platform) return current;
  return { platform: current.platform, channelId: status.id };
}

/** Trimmed owner check: does this row currently belong to the given owner? */
export function statusOwnedBy(status: IChannelPluginStatus, owner: ChannelOwnerQuery): boolean {
  const companionId = nonEmpty(owner.companionId);
  const publicAgentId = nonEmpty(owner.publicAgentId);
  if (publicAgentId) return nonEmpty(status.publicAgentId) === publicAgentId;
  if (companionId) return nonEmpty(status.companionId) === companionId;
  return false;
}

/** A row with no companion and no public-agent owner (a free, bindable bot). */
export function statusIsUnbound(status: IChannelPluginStatus): boolean {
  return !nonEmpty(status.companionId) && !nonEmpty(status.publicAgentId);
}
