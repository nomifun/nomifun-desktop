/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';

import BindTargetOptionRow from '@renderer/components/base/BindTargetOptionRow';
import type { TChatConversation } from '@/common/config/storage';

/**
 * Renders a two-line option node for the cron "specified conversation" Select:
 * - Line 1: conversation name (or `#N` id fallback) + a dimmed backend/type badge.
 * - Line 2: the conversation workspace path (middle-truncated) followed by `#N`.
 *
 * Conversation id is an INTEGER primary key, so the full id is just `#N` — no
 * prefix-strip (shortSessionId) is needed here.
 */
export const renderConversationOption = (conv: TChatConversation): React.ReactNode => {
  const idLabel = `#${conv.id}`;
  const extra = conv.extra as unknown as { workspace?: string; backend?: string } | undefined;
  return <BindTargetOptionRow title={conv.name || idLabel} badge={extra?.backend || conv.type} path={extra?.workspace} idLabel={idLabel} />;
};
