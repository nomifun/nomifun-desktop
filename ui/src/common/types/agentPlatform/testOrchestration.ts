import type {
  AgentPresetDraft,
  AgentResourceSelection,
  CreateAgentSessionRequest,
  CreateAgentSessionResponse,
  CreateAgentSessionTurnResponse,
  ResolveAgentPresetPreviewResponse,
  SaveAgentPresetRevisionRequest,
  SaveAgentPresetRevisionResponse,
} from './contracts';

export interface AgentPresetTestPorts {
  preview(draft: AgentPresetDraft): Promise<ResolveAgentPresetPreviewResponse>;
  save(request: SaveAgentPresetRevisionRequest): Promise<SaveAgentPresetRevisionResponse>;
  createSession(request: CreateAgentSessionRequest): Promise<CreateAgentSessionResponse>;
  createTurn(
    sessionId: CreateAgentSessionResponse['agent_session_id'],
    input: string,
    idempotencyKey: string
  ): Promise<CreateAgentSessionTurnResponse>;
}

export interface RunAgentPresetTestInput {
  draft: AgentPresetDraft;
  dirty: boolean;
  input: string;
  idempotencyKey: string;
  resourceSelections: AgentResourceSelection[];
  ports: AgentPresetTestPorts;
}

export interface RunAgentPresetTestResult {
  preview: ResolveAgentPresetPreviewResponse;
  savedRevision?: SaveAgentPresetRevisionResponse;
  session: CreateAgentSessionResponse;
  turn: CreateAgentSessionTurnResponse;
}

/**
 * D-022 client orchestration. There is deliberately no backend Test endpoint:
 * dirty drafts use ordinary Save Revision, then both clean and dirty paths use
 * the ordinary persistent AgentSession and Turn APIs with real resources.
 */
export async function runAgentPresetTest(
  input: RunAgentPresetTestInput
): Promise<RunAgentPresetTestResult> {
  const preview = await input.ports.preview(input.draft);
  if (!preview.can_create_session || preview.status !== 'ready') {
    throw new Error(preview.diagnostics[0]?.code ?? 'PRESET_REVISION_SAVE_FAILED');
  }

  const currentRevision = input.draft.current_revision;
  const previewRequiresSave =
    input.dirty ||
    !currentRevision ||
    currentRevision.preset_id !== preview.candidate_revision_ref.preset_id ||
    currentRevision.revision !== preview.candidate_revision_ref.revision ||
    currentRevision.revision_digest !== preview.candidate_revision_ref.revision_digest;

  let savedRevision: SaveAgentPresetRevisionResponse | undefined;
  if (previewRequiresSave) {
    savedRevision = await input.ports.save({
      expected_current_revision: input.draft.current_revision,
      preview_digest: preview.preview_digest,
      draft: input.draft,
      reason: 'Agent Settings Test',
    });
  }

  const session = await input.ports.createSession({
    preset_id: input.draft.preset_id,
    title: `${input.draft.display_name} Test`,
    ...(input.resourceSelections.length > 0 ? { resource_selections: input.resourceSelections } : {}),
  });
  const turn = await input.ports.createTurn(
    session.agent_session_id,
    input.input,
    input.idempotencyKey
  );
  return { preview, savedRevision, session, turn };
}
