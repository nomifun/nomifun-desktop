/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import { parseSshHostId, type SshHostId } from '@/common/types/ids';

/** The current canonical resource binding is the sole source of SSH identity.
 * The host book supplies labels and connection details, never session ownership. */
export const conversationSshHostId = (
  conversation?: Pick<TChatConversation, 'agent_snapshot'> | null,
): SshHostId | undefined => {
  const binding = conversation?.agent_snapshot?.canonical_binding?.typed_resource_bindings.find(
    (resource) => resource.resource_kind === 'ssh_host',
  );
  return binding ? parseSshHostId(binding.resource_id) : undefined;
};
