// src/common/types/orchestrator/orchestratorTypes.ts
// 「智能编排」(orchestration) wire types — hand-written mirrors of the backend
// api-types DTOs (Task 4). Field names are kept snake_case to match the JSON
// wire exactly, consistent with the rest of the codebase's wire types.
//
// IDs are STRINGS (`fleet_…`, `fmem_…`, `ows_…`), NOT i64. Numeric fields
// (max_parallel / sort_order / created_at / updated_at) are i64 on the backend
// but arrive as plain `number` over JSON, so they are typed `number` here.

/** A member's declared capability profile, used by the orchestrator for routing. */
export type TCapabilityProfile = {
  strengths: string[];
  modalities: string[];
  tools: boolean;
  reasoning: string;
  cost_tier: string;
  speed_tier: string;
};

/** Per-member execution constraints. */
export type TMemberConstraints = {
  max_concurrency?: number;
  cost_tier?: string;
  allowed_task_kinds?: string[];
};

/** A single agent slot within a fleet. */
export type TFleetMember = {
  id: string;
  agent_id: string;
  provider_id?: string;
  model?: string;
  role_hint?: string;
  capability_profile?: TCapabilityProfile;
  constraints?: TMemberConstraints;
  sort_order: number;
};

/** A persisted fleet (group of agents) record. */
export type TFleet = {
  id: string;
  name: string;
  description?: string;
  max_parallel?: number;
  members: TFleetMember[];
  created_at: number;
  updated_at: number;
};

/** A persisted orchestration workspace record. */
export type TOrchWorkspace = {
  id: string;
  name: string;
  default_fleet_id?: string;
  workspace_dir?: string;
  created_at: number;
  updated_at: number;
};

// ── Request payloads ────────────────────────────────────────────────────────

/** Input shape for a fleet member when creating/updating a fleet. */
export type TFleetMemberInput = {
  agent_id: string;
  provider_id?: string;
  model?: string;
  role_hint?: string;
  capability_profile?: TCapabilityProfile;
  constraints?: TMemberConstraints;
  sort_order?: number;
};

/** Body for `POST /api/orchestrator/fleets`. */
export type TCreateFleet = {
  name: string;
  description?: string;
  max_parallel?: number;
  members: TFleetMemberInput[];
};

/** Body for `PUT /api/orchestrator/fleets/{id}` (all fields optional / partial). */
export type TUpdateFleet = {
  name?: string;
  description?: string;
  max_parallel?: number;
  members?: TFleetMemberInput[];
};

/** Body for `POST /api/orchestrator/workspaces`. */
export type TCreateWorkspace = {
  name: string;
  default_fleet_id?: string;
  workspace_dir?: string;
};

/** Body for `PUT /api/orchestrator/workspaces/{id}` (partial). */
export type TUpdateWorkspace = {
  name?: string;
  default_fleet_id?: string;
};

// ── Run engine ───────────────────────────────────────────────────────────────

/** Inferred task profile used by the orchestrator for member routing. */
export type TTaskProfile = {
  kind: string;
  needs_vision: boolean;
  needs_long_context: boolean;
  needs_high_reasoning: boolean;
  bulk: boolean;
};

/** A persisted orchestration run record. */
export type TRun = {
  id: string;
  workspace_id: string;
  goal: string;
  autonomy: string;
  max_parallel?: number;
  status: string;
  summary?: string;
  lead_conv_id?: number;
  total_tokens?: number;
  created_at: number;
  updated_at: number;
};

/** A single task within a run's plan (DAG node). */
export type TRunTask = {
  id: string;
  run_id: string;
  title: string;
  spec: string;
  task_profile?: TTaskProfile;
  status: string;
  conversation_id?: number;
  output_summary?: string;
  output_files: string[];
  attempt: number;
  tokens?: number;
  graph_x?: number;
  graph_y?: number;
};

/** A dependency edge between two run tasks (blocker → blocked). */
export type TRunTaskDep = {
  blocker_task_id: string;
  blocked_task_id: string;
};

/** An assignment of a task to a fleet member (worker). */
export type TAssignment = {
  id: string;
  task_id: string;
  member_id: string;
  score?: number;
  rationale?: string;
  source: string;
  locked: boolean;
};

/** Full run detail: the run plus its plan (tasks/deps) and assignments. */
export type TRunDetail = {
  run: TRun;
  tasks: TRunTask[];
  deps: TRunTaskDep[];
  assignments: TAssignment[];
};

// ── Request payloads ─────────────────────────────────────────────────────────

/** Body for `POST /api/orchestrator/runs`. */
export type TCreateRun = {
  workspace_id: string;
  goal: string;
  fleet_id: string;
  autonomy?: string;
  max_parallel?: number;
};
