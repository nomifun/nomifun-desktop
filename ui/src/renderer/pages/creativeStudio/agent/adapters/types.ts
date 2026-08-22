/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  IConversationTurnCompletedEvent,
  IConversationTurnStartedEvent,
  IResponseMessage,
  ISendMessageResult,
} from '@/common/adapter/ipcBridge';
import type { ConversationId, MessageId } from '@/common/types/ids';

import type { CreativeModelSelectionRef } from '../../models';
import type { CreativeStudioAgentMessage } from '../types';

export interface NomiCreativeStudioAgentSessionResolutionInput {
  projectId: string;
  sessionId: string;
  model: CreativeModelSelectionRef;
  pendingTurnIdempotencyKey: string | null;
  signal: AbortSignal;
}
/**
 * A resolver must return a dedicated Nomi conversation plus the server-owned
 * persisted history and its exact `historyKey` proof. Shared conversations are rejected so
 * stopping a Creative Studio turn can never cancel an unrelated user turn.
 */
export interface NomiCreativeStudioAgentSessionBinding {
  ownership: 'creative-studio-exclusive';
  projectId: string;
  sessionId: string;
  conversationId: ConversationId;
  model: CreativeModelSelectionRef;
  historyKey: string;
}

export interface NomiCreativeStudioAgentSessionResolution {
  binding: NomiCreativeStudioAgentSessionBinding;
  history: readonly CreativeStudioAgentMessage[];
  /** Durable assistant proposals already committed to the canonical project. */
  appliedProposalMessageIds: readonly MessageId[];
  created: boolean;
}

export type NomiCreativeStudioAgentSessionResolver = (
  input: NomiCreativeStudioAgentSessionResolutionInput
) => Promise<NomiCreativeStudioAgentSessionResolution>;

export type NomiConversationRuntimeAuthority = 'idle' | 'processing' | 'unknown';

export interface NomiCreativeStudioConversationSnapshot {
  conversationId: ConversationId;
  model: CreativeModelSelectionRef;
  authority: NomiConversationRuntimeAuthority;
  activeTurnId?: MessageId;
}

export interface NomiCreativeStudioAgentTransport {
  inspect(conversationId: ConversationId): Promise<NomiCreativeStudioConversationSnapshot>;
  sendMessage(input: {
    conversationId: ConversationId;
    modelInput: string;
    skillIds: readonly string[];
    idempotencyKey: string;
  }): Promise<ISendMessageResult>;
  stopAndConfirm(conversationId: ConversationId): Promise<void>;
  onResponse(listener: (event: IResponseMessage) => void): () => void;
  onTurnStarted(listener: (event: IConversationTurnStartedEvent) => void): () => void;
  onTurnCompleted(listener: (event: IConversationTurnCompletedEvent) => void): () => void;
  onReconnected(listener: () => void): () => void;
}

export interface NomiCreativeStudioAgentPortOptions {
  resolveSession: NomiCreativeStudioAgentSessionResolver;
  transport?: NomiCreativeStudioAgentTransport;
  turnStartTimeoutMs?: number;
  recoveryPollMs?: number;
}
