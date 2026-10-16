/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageText } from '@/common/chat/chatLib';

/** An exact presentation source, shared by live ingress and canonical views. */
export function proseSourceIdentity(message: IMessageText): string | undefined {
  const source = message.message_id ?? message.msg_id;
  if (!source || message.position !== 'left') return undefined;
  return JSON.stringify([message.conversation_id, message.turn_id ?? null, source]);
}
