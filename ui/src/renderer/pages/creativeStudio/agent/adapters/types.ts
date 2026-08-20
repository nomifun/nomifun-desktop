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
  history: readonly CreativeStudioAgentMessage[];
  historyKey: string;
  signal: AbortSignal;
}
/**
 * A resolver must return a dedicated Nomi conversation whose persisted history
 * maps exactly to `historyKey`. Shared main-chat conversations are rejected so
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

export type NomiCreativeStudioAgentSessionResolver = (
  input: NomiCreativeStudioAgentSessionResolutionInput
) => Promise<NomiCreativeStudioAgentSessionBinding>;

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
    prompt: string;
    idempotencyKey: string;
  }): Promise<ISendMessageResult>;
  stopAndConfirm(conversationId: ConversationId): Promise<void>;
  createIdempotencyKey(): string;
  onResponse(listener: (event: IResponseMessage) => void): () => void;
  onTurnStarted(listener: (event: IConversationTurnStartedEvent) => void): () => void;
  onTurnCompleted(listener: (event: IConversationTurnCompletedEvent) => void): () => void;
  onReconnected(listener: () => void): () => void;
}

export interface NomiCreativeStudioAgentPortOptions {
  resolveSession: NomiCreativeStudioAgentSessionResolver;
  transport?: NomiCreativeStudioAgentTransport;
  turnStartTimeoutMs?: number;
}
