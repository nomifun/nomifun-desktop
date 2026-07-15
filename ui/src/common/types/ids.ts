/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Opaque entity identifiers.
 *
 * Every persistent entity ID is a canonical prefixed UUIDv7 string at storage
 * and protocol boundaries. The brand prevents accidentally passing one
 * entity's identifier to another entity's API without runtime wrappers.
 */
declare const entityIdBrand: unique symbol;

export type EntityId<Kind extends string> = string & {
  readonly [entityIdBrand]: Kind;
};

export type EntityKind =
  | 'conversation'
  | 'terminal'
  | 'requirement'
  | 'artifact'
  | 'mcp-server'
  | 'remote-agent'
  | 'webhook'
  | 'knowledge-base'
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
  | 'execution-event'
  | 'execution-link'
  | 'companion'
  | 'companion-memory'
  | 'companion-suggestion'
  | 'companion-learn-run'
  | 'companion-session-window'
  | 'figure'
  | 'public-agent-audit-entry'
  | 'companion-evolution-feedback'
  | 'public-agent'
  | 'channel'
  | 'channel-user'
  | 'channel-session'
  | 'attachment'
  | 'connector-credential'
  | 'idmm-intervention'
  | 'user'
  | 'canvas'
  | 'asset'
  | 'creation-task'
  | 'workshop-node'
  | 'workshop-edge';

export type ConversationId = EntityId<'conversation'>;
export type TerminalId = EntityId<'terminal'>;
export type RequirementId = EntityId<'requirement'>;
export type ArtifactId = EntityId<'artifact'>;
export type McpServerId = EntityId<'mcp-server'>;
export type RemoteAgentId = EntityId<'remote-agent'>;
export type WebhookId = EntityId<'webhook'>;
export type KnowledgeBaseId = EntityId<'knowledge-base'>;
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
export type ExecutionEventId = EntityId<'execution-event'>;
export type ExecutionLinkId = EntityId<'execution-link'>;
export type CompanionId = EntityId<'companion'>;
export type CompanionMemoryId = EntityId<'companion-memory'>;
export type CompanionSuggestionId = EntityId<'companion-suggestion'>;
export type CompanionLearnRunId = EntityId<'companion-learn-run'>;
export type CompanionSessionWindowId = EntityId<'companion-session-window'>;
export type FigureId = EntityId<'figure'>;
export type PublicAgentAuditEntryId = EntityId<'public-agent-audit-entry'>;
export type CompanionEvolutionFeedbackId = EntityId<'companion-evolution-feedback'>;
export type PublicAgentId = EntityId<'public-agent'>;
export type ChannelId = EntityId<'channel'>;
export type ChannelUserId = EntityId<'channel-user'>;
export type ChannelSessionId = EntityId<'channel-session'>;
export type AttachmentId = EntityId<'attachment'>;
export type ConnectorCredentialId = EntityId<'connector-credential'>;
export type IdmmInterventionId = EntityId<'idmm-intervention'>;
export type UserId = EntityId<'user'>;
export type CanvasId = EntityId<'canvas'>;
export type AssetId = EntityId<'asset'>;
export type CreationTaskId = EntityId<'creation-task'>;
export type WorkshopNodeId = EntityId<'workshop-node'>;
export type WorkshopEdgeId = EntityId<'workshop-edge'>;

export class InvalidEntityIdError extends TypeError {
  readonly entityKind: string;
  readonly value: unknown;

  constructor(entityKind: string, value: unknown) {
    super(`Invalid ${entityKind} id: expected its canonical prefixed UUIDv7 string`);
    this.name = 'InvalidEntityIdError';
    this.entityKind = entityKind;
    this.value = value;
  }
}

const ENTITY_ID_PREFIXES = {
  conversation: 'conv',
  terminal: 'term',
  requirement: 'req',
  artifact: 'artifact',
  'mcp-server': 'mcp',
  'remote-agent': 'ragent',
  webhook: 'webhook',
  'knowledge-base': 'kb',
  'knowledge-binding': 'kbind',
  provider: 'prov',
  agent: 'agent',
  preset: 'preset',
  'preset-tag': 'presettag',
  message: 'msg',
  'cron-job': 'cron',
  'cron-job-run': 'cronrun',
  'execution-template': 'aext',
  'execution-template-participant': 'aetp',
  execution: 'exec',
  'execution-participant': 'execpart',
  'execution-step': 'execstep',
  'execution-attempt': 'eattempt',
  'execution-event': 'aevt',
  'execution-link': 'execlink',
  companion: 'companion',
  'companion-memory': 'mem',
  'companion-suggestion': 'sug',
  'companion-learn-run': 'plr',
  'companion-session-window': 'csw',
  figure: 'figure',
  'public-agent-audit-entry': 'audit',
  'companion-evolution-feedback': 'evf',
  'public-agent': 'pubagent',
  channel: 'chn',
  'channel-user': 'chu',
  'channel-session': 'chs',
  attachment: 'att',
  'connector-credential': 'conn',
  'idmm-intervention': 'idmmrec',
  user: 'user',
  canvas: 'wsc',
  asset: 'wsa',
  'creation-task': 'wst',
  'workshop-node': 'wsn',
  'workshop-edge': 'wse',
} as const satisfies Record<EntityKind, string>;

const CANONICAL_UUID_V7 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

/**
 * Strictly validates an ID received at a route, wire, or storage boundary.
 * Numbers are intentionally rejected rather than stringified implicitly.
 */
export function parseEntityId<Kind extends EntityKind>(kind: Kind, value: unknown): EntityId<Kind> {
  const prefix = ENTITY_ID_PREFIXES[kind];
  if (
    typeof value !== 'string' ||
    value.trim() !== value ||
    !value.startsWith(`${prefix}_`) ||
    !CANONICAL_UUID_V7.test(value.slice(prefix.length + 1))
  ) {
    throw new InvalidEntityIdError(kind, value);
  }
  return value as EntityId<Kind>;
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
export const parseArtifactId = (value: unknown): ArtifactId => parseEntityId('artifact', value);
export const parseMcpServerId = (value: unknown): McpServerId =>
  parseEntityId('mcp-server', value);
export const parseRemoteAgentId = (value: unknown): RemoteAgentId =>
  parseEntityId('remote-agent', value);
export const parseWebhookId = (value: unknown): WebhookId => parseEntityId('webhook', value);
export const parseKnowledgeBaseId = (value: unknown): KnowledgeBaseId =>
  parseEntityId('knowledge-base', value);
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
export const parseExecutionEventId = (value: unknown): ExecutionEventId =>
  parseEntityId('execution-event', value);
export const parseExecutionLinkId = (value: unknown): ExecutionLinkId =>
  parseEntityId('execution-link', value);
export const parseCompanionId = (value: unknown): CompanionId => parseEntityId('companion', value);
export const parseCompanionMemoryId = (value: unknown): CompanionMemoryId =>
  parseEntityId('companion-memory', value);
export const parseCompanionSuggestionId = (value: unknown): CompanionSuggestionId =>
  parseEntityId('companion-suggestion', value);
export const parseCompanionLearnRunId = (value: unknown): CompanionLearnRunId =>
  parseEntityId('companion-learn-run', value);
export const parseCompanionSessionWindowId = (value: unknown): CompanionSessionWindowId =>
  parseEntityId('companion-session-window', value);
export const parseFigureId = (value: unknown): FigureId => parseEntityId('figure', value);
export const parsePublicAgentAuditEntryId = (value: unknown): PublicAgentAuditEntryId =>
  parseEntityId('public-agent-audit-entry', value);
export const parseCompanionEvolutionFeedbackId = (
  value: unknown
): CompanionEvolutionFeedbackId => parseEntityId('companion-evolution-feedback', value);
export const parsePublicAgentId = (value: unknown): PublicAgentId =>
  parseEntityId('public-agent', value);
export const parseChannelId = (value: unknown): ChannelId => parseEntityId('channel', value);
export const parseChannelUserId = (value: unknown): ChannelUserId =>
  parseEntityId('channel-user', value);
export const parseChannelSessionId = (value: unknown): ChannelSessionId =>
  parseEntityId('channel-session', value);
export const parseAttachmentId = (value: unknown): AttachmentId =>
  parseEntityId('attachment', value);
export const parseConnectorCredentialId = (value: unknown): ConnectorCredentialId =>
  parseEntityId('connector-credential', value);
export const parseIdmmInterventionId = (value: unknown): IdmmInterventionId =>
  parseEntityId('idmm-intervention', value);
export const parseUserId = (value: unknown): UserId => parseEntityId('user', value);
export const parseCanvasId = (value: unknown): CanvasId => parseEntityId('canvas', value);
export const parseAssetId = (value: unknown): AssetId => parseEntityId('asset', value);
export const parseCreationTaskId = (value: unknown): CreationTaskId =>
  parseEntityId('creation-task', value);
export const parseWorkshopNodeId = (value: unknown): WorkshopNodeId =>
  parseEntityId('workshop-node', value);
export const parseWorkshopEdgeId = (value: unknown): WorkshopEdgeId =>
  parseEntityId('workshop-edge', value);

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
