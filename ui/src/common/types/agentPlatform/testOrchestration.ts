import type {
  AgentBindingValue,
  AgentPresetDraft,
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
  ports: AgentPresetTestPorts;
}

export interface RunAgentPresetTestResult {
  preview: ResolveAgentPresetPreviewResponse;
  savedRevision?: SaveAgentPresetRevisionResponse;
  session: CreateAgentSessionResponse;
  turn: CreateAgentSessionTurnResponse;
}

const bindingFromPreview = (
  draft: AgentPresetDraft,
  preview: ResolveAgentPresetPreviewResponse,
  saved?: SaveAgentPresetRevisionResponse
): AgentBindingValue => {
  const presetRevision = saved?.revision.reference ?? preview.candidate_revision_ref;
  const snapshot = saved?.resolved_snapshot_ref ?? preview.resolved_snapshot_ref;
  if (!snapshot) throw new Error('PREVIEW_SNAPSHOT_MISSING');
  return {
    preset_revision_ref: presetRevision,
    resolved_snapshot_ref: snapshot,
    typed_resource_bindings: draft.document.resource_bindings,
    binding_version: 1,
  };
};

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

  let savedRevision: SaveAgentPresetRevisionResponse | undefined;
  if (input.dirty) {
    savedRevision = await input.ports.save({
      expected_current_revision: input.draft.current_revision,
      preview_digest: preview.preview_digest,
      draft: input.draft,
      reason: 'Agent Settings Test',
    });
  } else if (!input.draft.current_revision) {
    throw new Error('CLEAN_DRAFT_REVISION_MISSING');
  }

  const agentBinding = bindingFromPreview(input.draft, preview, savedRevision);
  const session = await input.ports.createSession({
    agent_binding: agentBinding,
    title: `${input.draft.display_name} Test`,
  });
  const turn = await input.ports.createTurn(
    session.agent_session_id,
    input.input,
    input.idempotencyKey
  );
  return { preview, savedRevision, session, turn };
}
