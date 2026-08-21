/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeModelSelectionRef } from '../models';

export type CreativeStudioAgentView = 'chat' | 'history';
export type CreativeStudioAgentPanelLoadState = 'loading' | 'ready' | 'failed';

export interface CreativeStudioAgentSessionSummary {
  id: string;
  title: string;
  messageCount: number;
  updatedAtLabel?: string;
}
interface CreativeStudioAgentMessageBase {
  id: string;
  text: string;
}

export interface CreativeStudioAgentUserMessage extends CreativeStudioAgentMessageBase {
  role: 'user';
  status: 'complete';
}

export interface CreativeStudioAgentCompleteMessage extends CreativeStudioAgentMessageBase {
  role: 'assistant';
  status: 'complete';
}

export interface CreativeStudioAgentRunningMessage extends CreativeStudioAgentMessageBase {
  role: 'assistant';
  status: 'running';
  activityLabel?: string;
}

export interface CreativeStudioAgentFailedMessage extends CreativeStudioAgentMessageBase {
  role: 'assistant';
  status: 'failed';
  errorMessage: string;
}

export interface CreativeStudioAgentStoppedMessage extends CreativeStudioAgentMessageBase {
  role: 'assistant';
  status: 'stopped';
}

export type CreativeStudioAgentMessage =
  | CreativeStudioAgentUserMessage
  | CreativeStudioAgentCompleteMessage
  | CreativeStudioAgentRunningMessage
  | CreativeStudioAgentFailedMessage
  | CreativeStudioAgentStoppedMessage;

export interface CreativeStudioAgentSendInput {
  prompt: string;
  model: CreativeModelSelectionRef;
  /** Exact context chips still included when the user submits this turn. */
  contextNodeIds: string[];
  /** Ordered explicit NomiFun Skills; never inferred from prompt text. */
  skillIds: string[];
}

export interface CreativeStudioAgentContextItem {
  id: string;
  label: string;
  type: string;
  selected: boolean;
}

export interface CreativeStudioAgentSkillOption {
  id: string;
  label: string;
  description: string;
}

/**
 * Controlled panel contract. Conversation persistence, streaming and canvas
 * actions stay with the caller; this surface only presents explicit state and
 * emits user intent.
 */
export interface CreativeStudioAgentPanelProps {
  view: CreativeStudioAgentView;
  loadState: CreativeStudioAgentPanelLoadState;
  sessions: readonly CreativeStudioAgentSessionSummary[];
  activeSessionId: string | null;
  messages: readonly CreativeStudioAgentMessage[];
  draft: string;
  model: CreativeModelSelectionRef | null;
  contextItems: readonly CreativeStudioAgentContextItem[];
  skillOptions: readonly CreativeStudioAgentSkillOption[];
  selectedSkillIds: readonly string[];
  /** A dedicated NomiFun conversation cannot change model after its first turn. */
  modelLocked?: boolean;
  isRunning: boolean;
  errorMessage?: string;
  disabled?: boolean;
  onViewChange(view: CreativeStudioAgentView): void;
  onNewSession(): void;
  onSelectSession(sessionId: string): void;
  onDraftChange(draft: string): void;
  onModelChange(model: CreativeModelSelectionRef): void;
  onRemoveContextItem(itemId: string): void;
  onToggleSkill(skillId: string): void;
  onSend(input: CreativeStudioAgentSendInput): void;
  onStop(): void;
  onCollapse(): void;
  onRetryLoad?(): void;
  onRetryMessage?(messageId: string): void;
  onOpenModelSettings?(): void;
}
