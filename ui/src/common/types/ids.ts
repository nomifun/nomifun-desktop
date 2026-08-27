/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * ID boundary types.
 *
 * Stable business IDs are canonical lowercase UUIDv7 strings without a
 * business prefix. SQLite autoincrement keys never cross this product-facing
 * boundary.
 */
declare const entityIdBrand: unique symbol;

export type EntityId<Kind extends string> = string & {
  readonly [entityIdBrand]: Kind;
};

export type EntityKind =
  | 'conversation'
  | 'terminal'
  | 'remote-agent'
  | 'ssh-host'
  | 'webhook'
  | 'knowledge-base'
  | 'knowledge-entry'
  | 'knowledge-binding'
  | 'provider'
  | 'agent'
  | 'preset'
  | 'preset-tag'
  | 'message'
  | 'cron-job'
  | 'cron-job-run'
  | 'execution-template'
  | 'execution-template-participant'
  | 'execution'
  | 'execution-participant'
  | 'execution-step'
  | 'execution-attempt'
  | 'companion'
  | 'companion-event'
  | 'companion-skill'
  | 'companion-memory'
  | 'companion-session-window'
  | 'skill-pattern'
  | 'figure'
  | 'companion-evolution-feedback'
  | 'cs-agent'
  | 'cs-dialogue'
  | 'cs-message'
  | 'cs-note'
  | 'channel-plugin'
  | 'channel-user'
  | 'channel-session'
  | 'attachment'
  | 'preview-snapshot'
  | 'conversation-artifact'
  | 'mcp-server'
  | 'idmm-intervention'
  | 'requirement'
  | 'persisted-artifact'
  | 'user'
  | 'asset'
  | 'creation-task'
  | 'creative-studio-project'
  | 'creative-studio-node'
  | 'creative-studio-connection'
  | 'miniapp';

export type ConversationId = EntityId<'conversation'>;
export type TerminalId = EntityId<'terminal'>;
export type RequirementId = EntityId<'requirement'>;
export type ConversationArtifactId = EntityId<'conversation-artifact'>;
export type McpServerId = EntityId<'mcp-server'>;
export type RemoteAgentId = EntityId<'remote-agent'>;
export type SshHostId = EntityId<'ssh-host'>;
export type WebhookId = EntityId<'webhook'>;
export type KnowledgeBaseId = EntityId<'knowledge-base'>;
export type KnowledgeEntryId = EntityId<'knowledge-entry'>;
export type KnowledgeBindingId = EntityId<'knowledge-binding'>;
export type ProviderId = EntityId<'provider'>;
export type AgentId = EntityId<'agent'>;
export type PresetId = EntityId<'preset'>;
export type PresetTagId = EntityId<'preset-tag'>;
export type MessageId = EntityId<'message'>;
export type CronJobId = EntityId<'cron-job'>;
export type CronJobRunId = EntityId<'cron-job-run'>;
export type ExecutionTemplateId = EntityId<'execution-template'>;
export type ExecutionTemplateParticipantId = EntityId<'execution-template-participant'>;
export type ExecutionId = EntityId<'execution'>;
export type ExecutionParticipantId = EntityId<'execution-participant'>;
export type ExecutionStepId = EntityId<'execution-step'>;
export type ExecutionAttemptId = EntityId<'execution-attempt'>;
export type CompanionId = EntityId<'companion'>;
export type CompanionEventId = EntityId<'companion-event'>;
export type CompanionSkillId = EntityId<'companion-skill'>;
export type CompanionMemoryId = EntityId<'companion-memory'>;
export type CompanionSessionWindowId = EntityId<'companion-session-window'>;
export type SkillPatternId = EntityId<'skill-pattern'>;
export type FigureId = EntityId<'figure'>;
export type CompanionEvolutionFeedbackId = EntityId<'companion-evolution-feedback'>;
export type CsAgentId = EntityId<'cs-agent'>;
export type CsDialogueId = EntityId<'cs-dialogue'>;
export type CsMessageId = EntityId<'cs-message'>;
export type CsNoteId = EntityId<'cs-note'>;
export type ChannelPluginId = EntityId<'channel-plugin'>;
export type ChannelUserId = EntityId<'channel-user'>;
export type ChannelSessionId = EntityId<'channel-session'>;
export type AttachmentId = EntityId<'attachment'>;
export type PreviewSnapshotId = EntityId<'preview-snapshot'>;
export type PersistedArtifactId = EntityId<'persisted-artifact'>;
export type IdmmInterventionId = EntityId<'idmm-intervention'>;
export type UserId = EntityId<'user'>;
export type AssetId = EntityId<'asset'>;
export type CreationTaskId = EntityId<'creation-task'>;
export type CreativeStudioProjectId = EntityId<'creative-studio-project'>;
export type CreativeStudioNodeId = EntityId<'creative-studio-node'>;
export type CreativeStudioConnectionId = EntityId<'creative-studio-connection'>;
export type MiniAppId = EntityId<'miniapp'>;

export class InvalidEntityIdError extends TypeError {
  readonly entityKind: string;
  readonly value: unknown;

  constructor(entityKind: string, value: unknown) {
    super(
      `Invalid ${entityKind} id: expected a canonical lowercase 36-character UUIDv7 without a prefix; legacy prefixed IDs are not accepted`,
    );
    this.name = 'InvalidEntityIdError';
    this.entityKind = entityKind;
    this.value = value;
  }
}

export const CANONICAL_UUID_V7 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

/**
 * Strictly validates a stable business ID received at a wire or storage
 * boundary. Numbers, prefixed v2 IDs, uppercase, whitespace and non-v7 UUIDs
 * are intentionally rejected rather than normalized.
 */
export function parseEntityId<Kind extends EntityKind>(kind: Kind, value: unknown): EntityId<Kind> {
  if (typeof value !== 'string' || !CANONICAL_UUID_V7.test(value)) {
    throw new InvalidEntityIdError(kind, value);
  }
  return value as EntityId<Kind>;
}

/** Parse an optional boundary ID while keeping strict UUIDv7 validation for present values. */
export function parseOptionalEntityId<Kind extends EntityKind>(
  kind: Kind,
  value: unknown,
): EntityId<Kind> | undefined {
  return value == null ? undefined : parseEntityId(kind, value);
}

export function tryParseEntityId<Kind extends EntityKind>(kind: Kind, value: unknown): EntityId<Kind> | null {
  try {
    return parseEntityId(kind, value);
  } catch {
    return null;
  }
}

export const parseConversationId = (value: unknown): ConversationId =>
  parseEntityId('conversation', value);
export const parseTerminalId = (value: unknown): TerminalId => parseEntityId('terminal', value);
export const parseRequirementId = (value: unknown): RequirementId =>
  parseEntityId('requirement', value);
export const parseConversationArtifactId = (value: unknown): ConversationArtifactId =>
  parseEntityId('conversation-artifact', value);
export const parseMcpServerId = (value: unknown): McpServerId =>
  parseEntityId('mcp-server', value);
export const parseRemoteAgentId = (value: unknown): RemoteAgentId =>
  parseEntityId('remote-agent', value);
export const parseSshHostId = (value: unknown): SshHostId =>
  parseEntityId('ssh-host', value);
export const parseWebhookId = (value: unknown): WebhookId => parseEntityId('webhook', value);
export const parseKnowledgeBaseId = (value: unknown): KnowledgeBaseId =>
  parseEntityId('knowledge-base', value);
export const parseKnowledgeEntryId = (value: unknown): KnowledgeEntryId =>
  parseEntityId('knowledge-entry', value);
export const parseKnowledgeBindingId = (value: unknown): KnowledgeBindingId =>
  parseEntityId('knowledge-binding', value);
export const parseProviderId = (value: unknown): ProviderId => parseEntityId('provider', value);
export const parseAgentId = (value: unknown): AgentId => parseEntityId('agent', value);
export const parsePresetId = (value: unknown): PresetId => parseEntityId('preset', value);
export const parsePresetTagId = (value: unknown): PresetTagId =>
  parseEntityId('preset-tag', value);
export const parseMessageId = (value: unknown): MessageId => parseEntityId('message', value);
export const parseCronJobId = (value: unknown): CronJobId => parseEntityId('cron-job', value);
export const parseCronJobRunId = (value: unknown): CronJobRunId =>
  parseEntityId('cron-job-run', value);
export const parseExecutionTemplateId = (value: unknown): ExecutionTemplateId =>
  parseEntityId('execution-template', value);
export const parseExecutionTemplateParticipantId = (
  value: unknown
): ExecutionTemplateParticipantId => parseEntityId('execution-template-participant', value);
export const parseExecutionId = (value: unknown): ExecutionId => parseEntityId('execution', value);
export const parseExecutionParticipantId = (value: unknown): ExecutionParticipantId =>
  parseEntityId('execution-participant', value);
export const parseExecutionStepId = (value: unknown): ExecutionStepId =>
  parseEntityId('execution-step', value);
export const parseExecutionAttemptId = (value: unknown): ExecutionAttemptId =>
  parseEntityId('execution-attempt', value);
export const parseCompanionId = (value: unknown): CompanionId => parseEntityId('companion', value);
export const parseCompanionEventId = (value: unknown): CompanionEventId =>
  parseEntityId('companion-event', value);
export const parseCompanionSkillId = (value: unknown): CompanionSkillId =>
  parseEntityId('companion-skill', value);
export const parseCompanionMemoryId = (value: unknown): CompanionMemoryId =>
  parseEntityId('companion-memory', value);
export const parseCompanionSessionWindowId = (value: unknown): CompanionSessionWindowId =>
  parseEntityId('companion-session-window', value);
export const parseSkillPatternId = (value: unknown): SkillPatternId =>
  parseEntityId('skill-pattern', value);
export const parseFigureId = (value: unknown): FigureId => parseEntityId('figure', value);
export const parseCompanionEvolutionFeedbackId = (
  value: unknown
): CompanionEvolutionFeedbackId => parseEntityId('companion-evolution-feedback', value);
export const parseCsAgentId = (value: unknown): CsAgentId =>
  parseEntityId('cs-agent', value);
export const parseCsDialogueId = (value: unknown): CsDialogueId =>
  parseEntityId('cs-dialogue', value);
export const parseCsMessageId = (value: unknown): CsMessageId =>
  parseEntityId('cs-message', value);
export const parseCsNoteId = (value: unknown): CsNoteId =>
  parseEntityId('cs-note', value);
export const parseChannelPluginId = (value: unknown): ChannelPluginId =>
  parseEntityId('channel-plugin', value);
export const parseChannelUserId = (value: unknown): ChannelUserId =>
  parseEntityId('channel-user', value);
export const parseChannelSessionId = (value: unknown): ChannelSessionId =>
  parseEntityId('channel-session', value);
export const parseAttachmentId = (value: unknown): AttachmentId =>
  parseEntityId('attachment', value);
export const parsePreviewSnapshotId = (value: unknown): PreviewSnapshotId =>
  parseEntityId('preview-snapshot', value);
export const parsePersistedArtifactId = (value: unknown): PersistedArtifactId =>
  parseEntityId('persisted-artifact', value);
export const parseIdmmInterventionId = (value: unknown): IdmmInterventionId =>
  parseEntityId('idmm-intervention', value);
export const parseUserId = (value: unknown): UserId => parseEntityId('user', value);
export const parseAssetId = (value: unknown): AssetId => parseEntityId('asset', value);
export const parseCreationTaskId = (value: unknown): CreationTaskId =>
  parseEntityId('creation-task', value);
export const parseCreativeStudioProjectId = (value: unknown): CreativeStudioProjectId =>
  parseEntityId('creative-studio-project', value);
export const parseCreativeStudioNodeId = (value: unknown): CreativeStudioNodeId =>
  parseEntityId('creative-studio-node', value);
export const parseCreativeStudioConnectionId = (value: unknown): CreativeStudioConnectionId =>
  parseEntityId('creative-studio-connection', value);
export const parseMiniAppId = (value: unknown): MiniAppId => parseEntityId('miniapp', value);

export type SessionTarget =
  | { readonly kind: 'conversation'; readonly id: ConversationId }
  | { readonly kind: 'terminal'; readonly id: TerminalId };

export const conversationTarget = (value: unknown): SessionTarget => ({
  kind: 'conversation',
  id: parseConversationId(value),
});

export const terminalTarget = (value: unknown): SessionTarget => ({
  kind: 'terminal',
  id: parseTerminalId(value),
});

export function isSameSessionTarget(left: SessionTarget, right: SessionTarget): boolean {
  return left.kind === right.kind && left.id === right.id;
}
