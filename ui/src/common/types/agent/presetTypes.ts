/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

// Mirror of nomifun-api-types/src/preset.rs.
// Any shape change on either side requires a same-PR update on the other.

export type PresetSource = 'builtin' | 'user' | 'extension';
export type PresetTarget = 'conversation' | 'execution_step' | 'companion' | 'public_companion' | 'cron';

export interface AgentPreference {
  agent_id: string;
  required: boolean;
}

export interface ModelPreference {
  provider_id?: string;
  model: string;
  required: boolean;
}

export interface SkillBinding {
  skill_name: string;
  required: boolean;
}

export interface KnowledgeBaseBinding {
  knowledge_base_id: string;
  required: boolean;
}

export interface PresetKnowledgePolicy {
  enabled: boolean;
  mode: string;
  writeback: boolean;
  eagerness?: 'conservative' | 'aggressive';
  grounded: boolean;
}

export interface Preset {
  id: string;
  revision: number;
  source: PresetSource;
  source_key?: string;
  name: string;
  name_i18n: Record<string, string>;
  description?: string;
  description_i18n: Record<string, string>;
  routing_description?: string;
  instructions: string;
  instructions_i18n: Record<string, string>;
  avatar?: string;
  fallback_allowed: boolean;
  preferred_agent_id?: string;
  targets: PresetTarget[];
  agent_preferences: AgentPreference[];
  model_preferences: ModelPreference[];
  included_skills: SkillBinding[];
  excluded_auto_skills: string[];
  knowledge_policy: PresetKnowledgePolicy;
  knowledge_bases: KnowledgeBaseBinding[];
  examples: string[];
  examples_i18n: Record<string, string[]>;
  audience_tags: string[];
  scenario_tags: string[];
  enabled: boolean;
  auto_selectable: boolean;
  sort_order: number;
  last_used_at?: number;
}

export interface CreatePresetRequest {
  id?: string;
  name: string;
  description?: string;
  routing_description?: string;
  instructions?: string;
  avatar?: string;
  fallback_allowed?: boolean;
  targets?: PresetTarget[];
  agent_preferences?: AgentPreference[];
  model_preferences?: ModelPreference[];
  included_skills?: SkillBinding[];
  excluded_auto_skills?: string[];
  knowledge_policy?: PresetKnowledgePolicy;
  knowledge_bases?: KnowledgeBaseBinding[];
  examples?: string[];
  examples_i18n?: Record<string, string[]>;
  audience_tags?: string[];
  scenario_tags?: string[];
  name_i18n?: Record<string, string>;
  description_i18n?: Record<string, string>;
  instructions_i18n?: Record<string, string>;
}

export type UpdatePresetRequest = Partial<Omit<CreatePresetRequest, 'id'>>;

export interface SetPresetStateRequest {
  id: string;
  enabled?: boolean;
  auto_selectable?: boolean;
  sort_order?: number;
  last_used_at?: number;
  /** Empty string clears the per-user preference. */
  preferred_agent_id?: string;
}

export interface PresetOverrides {
  agent_id?: string;
  provider_id?: string;
  model?: string;
  instructions?: string;
  include_skills?: string[];
  exclude_skills?: string[];
  knowledge_policy?: PresetKnowledgePolicy;
  knowledge_base_ids?: string[];
}

export interface ResolvePresetRequest {
  id: string;
  target: PresetTarget;
  locale?: string;
  overrides?: PresetOverrides;
}

export interface ResolvedPresetSnapshot {
  preset_id: string;
  preset_revision: number;
  preset_name: string;
  target: PresetTarget;
  routing_description?: string;
  instructions: string;
  resolved_agent_id?: string;
  resolved_agent_type?: string;
  resolved_agent_backend?: string;
  resolved_model?: ModelPreference;
  included_skills: string[];
  excluded_auto_skills: string[];
  knowledge_policy: PresetKnowledgePolicy;
  knowledge_base_ids: string[];
  warnings: string[];
}

export interface ImportPresetsRequest {
  presets: CreatePresetRequest[];
}

export interface PresetImportError {
  id: string;
  error: string;
}

export interface ImportPresetsResult {
  imported: number;
  skipped: number;
  failed: number;
  errors: PresetImportError[];
}

export type PresetTagDimension = 'audience' | 'scenario';

export interface PresetTag {
  key: string;
  dimension: PresetTagDimension;
  label: string;
  label_i18n: Record<string, string>;
  sort_order: number;
  builtin: boolean;
}

export interface CreatePresetTagRequest {
  dimension: PresetTagDimension;
  label: string;
}

export interface UpdatePresetTagRequest {
  key: string;
  label?: string;
  sort_order?: number;
}
