import type { ConversationId, MessageId, ProviderId } from '@/common/types/ids';
import type { AgentPresetId } from '@/common/types/agentPlatform';
import type { GuidAgentSelectionPreference } from '@/common/config/configKeys';

export type CreationMode = 'image' | 'video' | 'music';
export type CreationCapability = 't2i' | 'i2i' | 'inpaint' | 't2v' | 'i2v' | 'music' | 'tts';
export type CreationInput = { asset_id: string; kind: 'image' | 'video' | 'audio' | 'text'; role: 'reference' | 'mask' | 'first_frame' | 'last_frame' | 'video' | 'audio' };
export type CreationParameters = Record<string, string | number | boolean | null>;
export interface CreationReference extends CreationInput { title: string; url?: string }
export interface GenerationModel { providerId: ProviderId; model: string }
export interface CreationDraft {
  mode: CreationMode | null;
  lastMode: CreationMode;
  models: Record<CreationMode, GenerationModel | null>;
  parameters: Record<CreationMode, CreationParameters>;
  references: CreationReference[];
  pendingPrompt?: string;
  pendingFiles?: string[];
  selectedAgent?: GuidAgentSelectionPreference;
  presetId?: AgentPresetId;
  agentLabel?: string;
}
export interface SubmitCreationRequest {
  provider_id: ProviderId;
  model: string;
  capability: CreationCapability;
  params: CreationParameters;
  inputs: CreationInput[];
  preset_id?: AgentPresetId;
  files?: string[];
}
export interface ConversationCreationTask {
  creation_task_id: string;
  owner: { kind: 'conversation_turn'; conversation_id: ConversationId; message_id: MessageId };
  provider_id: ProviderId;
  model: string;
  capability: CreationCapability;
  params: CreationParameters;
  inputs: CreationInput[] | null;
  status: 'queued' | 'running' | 'succeeded' | 'failed' | 'canceled';
  error: { message?: string; kind?: string } | null;
  result_asset_ids: string[];
  submitted_at: number;
  started_at: number | null;
  finished_at: number | null;
}
export interface CreationReceipt { message_id: MessageId; tasks: ConversationCreationTask[] }

export const creationModeFor = (capability: CreationCapability): CreationMode | null =>
  capability === 'tts' ? null : capability === 'music' ? 'music' : capability === 't2v' || capability === 'i2v' ? 'video' : 'image';

export const inputsForMode = (mode: CreationMode, references: readonly CreationInput[]): CreationInput[] =>
  mode === 'music' ? [] : references.filter(ref => ref.kind === 'image' && (mode !== 'video' || ref.role !== 'mask')).map(({ asset_id, kind, role }) => ({
    asset_id, kind,
    role: mode === 'image' ? (role === 'mask' ? 'mask' : 'reference') : (role === 'last_frame' ? 'last_frame' : role === 'first_frame' ? 'first_frame' : 'reference'),
  }));

/** The direct composer currently supports image references, not document/video/audio transformation. */
export const filesForCreation = (mode: CreationMode | null, files: readonly string[]): string[] =>
  mode && mode !== 'music' ? files.filter(path => /\.(?:png|jpe?g|webp|gif)$/i.test(path)) : [];

export function capabilityFor(mode: CreationMode, inputs: readonly CreationInput[], hasFiles = false): CreationCapability {
  if (mode === 'music') return 'music';
  if (mode === 'video') return inputs.length || hasFiles ? 'i2v' : 't2v';
  return inputs.some(input => input.role === 'mask') ? 'inpaint' : inputs.length || hasFiles ? 'i2i' : 't2i';
}
