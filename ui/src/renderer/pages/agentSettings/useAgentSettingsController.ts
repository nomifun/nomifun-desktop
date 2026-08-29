import { agentPlatform } from '@/common/adapter/ipcBridge';
import type {
  AgentCatalogResponse,
  AgentPresetDraft,
  AgentPresetEditorResponse,
  AgentPresetLibraryResponse,
  AgentPresetSummary,
  InstallationTokenStateResponse,
  OfficialPresetKey,
  OfficialPresetTemplate,
  ResolveAgentPresetPreviewResponse,
  TemplateResourceSelection,
} from '@/common/types/agentPlatform';
import {
  cloneDraft,
  isDraftDirty,
  runAgentPresetTest,
  type RunAgentPresetTestResult,
} from '@/common/types/agentPlatform';
import { useCallback, useEffect, useMemo, useState } from 'react';

type Selection =
  | { kind: 'template'; template: OfficialPresetTemplate }
  | { kind: 'preset'; preset: AgentPresetSummary }
  | null;

const emptyCatalog: AgentCatalogResponse = {
  capabilities: [],
  skills: [],
  mcp_tools: [],
};

const previewRequest = (draft: AgentPresetDraft) => ({
  expected_current_revision: draft.current_revision,
  draft,
  scene: 'agent_settings' as const,
  surface: 'desktop' as const,
  audience: 'owner' as const,
});

const idempotencyKey = (): string =>
  `agent-settings-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;

export function useAgentSettingsController() {
  const [library, setLibrary] = useState<AgentPresetLibraryResponse | null>(null);
  const [catalog, setCatalog] = useState<AgentCatalogResponse>(emptyCatalog);
  const [selection, setSelection] = useState<Selection>(null);
  const [editor, setEditor] = useState<AgentPresetEditorResponse | null>(null);
  const [draft, setDraftState] = useState<AgentPresetDraft | null>(null);
  const [savedDraft, setSavedDraft] = useState<AgentPresetDraft | null>(null);
  const [preview, setPreview] = useState<ResolveAgentPresetPreviewResponse | null>(null);
  const [testResult, setTestResult] = useState<RunAgentPresetTestResult | null>(null);
  const [tokenState, setTokenState] = useState<InstallationTokenStateResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [busyAction, setBusyAction] = useState<'preview' | 'save' | 'test' | 'fork' | 'create' | null>(
    null
  );
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [nextLibrary, capabilities, skills, mcpTools, nextTokenState] = await Promise.all([
        agentPlatform.library.invoke(),
        agentPlatform.capabilities.invoke(),
        agentPlatform.skills.invoke(),
        agentPlatform.mcpTools.invoke(),
        agentPlatform.installationToken.status.invoke().catch(() => null),
      ]);
      const nextCatalog = { capabilities, skills, mcp_tools: mcpTools };
      setLibrary(nextLibrary);
      setCatalog(nextCatalog);
      setTokenState(nextTokenState);
      setSelection((current) => {
        if (current) return current;
        const firstTemplate = nextLibrary.official_templates[0];
        return firstTemplate ? { kind: 'template', template: firstTemplate } : null;
      });
    } catch (loadError) {
      setError(String(loadError));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const openTemplate = useCallback((template: OfficialPresetTemplate) => {
    setSelection({ kind: 'template', template });
    setEditor(null);
    setDraftState(null);
    setSavedDraft(null);
    setPreview(null);
    setTestResult(null);
    setError(null);
  }, []);

  const openPreset = useCallback(async (preset: AgentPresetSummary) => {
    setSelection({ kind: 'preset', preset });
    setBusyAction('create');
    setError(null);
    try {
      const response = await agentPlatform.getEditor.invoke({ preset_id: preset.preset_id });
      const nextDraft = cloneDraft(response.draft);
      setEditor(response);
      setDraftState(nextDraft);
      setSavedDraft(cloneDraft(nextDraft));
      setPreview(null);
      setTestResult(null);
    } catch (openError) {
      setError(String(openError));
    } finally {
      setBusyAction(null);
    }
  }, []);

  const applyEditor = useCallback((response: AgentPresetEditorResponse) => {
    const nextDraft = cloneDraft(response.draft);
    setEditor(response);
    setDraftState(nextDraft);
    setSavedDraft(response.revision ? cloneDraft(nextDraft) : null);
    setSelection({ kind: 'preset', preset: response.preset });
    setPreview(null);
    setTestResult(null);
  }, []);

  const createPreset = useCallback(
    async (displayName: string) => {
      setBusyAction('create');
      setError(null);
      try {
        const response = await agentPlatform.createPreset.invoke({
          display_name: displayName,
        });
        applyEditor(response);
        await load();
        setSelection({ kind: 'preset', preset: response.preset });
      } catch (createError) {
        setError(String(createError));
      } finally {
        setBusyAction(null);
      }
    },
    [applyEditor, load]
  );

  const forkTemplate = useCallback(
    async (
      templateKey: OfficialPresetKey,
      displayName: string,
      resourceBindings: TemplateResourceSelection[],
      modelRouteRefs: Record<string, string>
    ) => {
      setBusyAction('fork');
      setError(null);
      try {
        const response = await agentPlatform.createFromTemplate.invoke({
          template_id: templateKey,
          request: {
            display_name: displayName,
            resource_bindings: resourceBindings,
            model_route_refs: modelRouteRefs,
          },
        });
        applyEditor(response);
        await load();
        setSelection({ kind: 'preset', preset: response.preset });
      } catch (forkError) {
        setError(String(forkError));
      } finally {
        setBusyAction(null);
      }
    },
    [applyEditor, load]
  );

  const setDraft = useCallback((next: AgentPresetDraft) => {
    setDraftState(next);
    setPreview(null);
    setTestResult(null);
  }, []);

  const runPreview = useCallback(async (): Promise<ResolveAgentPresetPreviewResponse | null> => {
    if (!draft) return null;
    setBusyAction('preview');
    setError(null);
    try {
      const response = await agentPlatform.resolvePreview.invoke({
        preset_id: draft.preset_id,
        request: previewRequest(draft),
      });
      setPreview(response);
      return response;
    } catch (previewError) {
      setError(String(previewError));
      return null;
    } finally {
      setBusyAction(null);
    }
  }, [draft]);

  const saveRevision = useCallback(async () => {
    if (!draft) return null;
    setBusyAction('save');
    setError(null);
    try {
      const freshPreview = await agentPlatform.resolvePreview.invoke({
        preset_id: draft.preset_id,
        request: previewRequest(draft),
      });
      setPreview(freshPreview);
      if (!freshPreview.can_save_revision) return null;
      const saved = await agentPlatform.saveRevision.invoke({
        preset_id: draft.preset_id,
        request: {
          expected_current_revision: draft.current_revision,
          preview_digest: freshPreview.preview_digest,
          draft,
        },
      });
      const nextDraft: AgentPresetDraft = {
        ...draft,
        current_revision: saved.revision.reference,
      };
      setDraftState(nextDraft);
      setSavedDraft(cloneDraft(nextDraft));
      setEditor((current) =>
        current
          ? {
              preset: saved.preset,
              revision: saved.revision,
              draft: nextDraft,
            }
          : current
      );
      await load();
      setSelection({ kind: 'preset', preset: saved.preset });
      return saved;
    } catch (saveError) {
      setError(String(saveError));
      return null;
    } finally {
      setBusyAction(null);
    }
  }, [draft, load]);

  const runTest = useCallback(
    async (input: string) => {
      if (!draft) return;
      setBusyAction('test');
      setError(null);
      try {
        const dirty = isDraftDirty(savedDraft, draft);
        const result = await runAgentPresetTest({
          draft,
          dirty,
          input,
          idempotencyKey: idempotencyKey(),
          ports: {
            preview: async (nextDraft) =>
              agentPlatform.resolvePreview.invoke({
                preset_id: nextDraft.preset_id,
                request: previewRequest(nextDraft),
              }),
            save: async (request) =>
              agentPlatform.saveRevision.invoke({
                preset_id: request.draft.preset_id,
                request,
              }),
            createSession: async (request) => agentPlatform.sessions.create.invoke(request),
            createTurn: async (agentSessionId, content, key) =>
              agentPlatform.sessions.createTurn.invoke({
                agent_session_id: agentSessionId,
                request: {
                  input: { content },
                  idempotency_key: key,
                },
              }),
          },
        });
        setPreview(result.preview);
        setTestResult(result);
        if (result.savedRevision) {
          const nextDraft = {
            ...draft,
            current_revision: result.savedRevision.revision.reference,
          };
          setDraftState(nextDraft);
          setSavedDraft(cloneDraft(nextDraft));
          setEditor({
            preset: result.savedRevision.preset,
            revision: result.savedRevision.revision,
            draft: nextDraft,
          });
          await load();
          setSelection({ kind: 'preset', preset: result.savedRevision.preset });
        }
      } catch (testError) {
        setError(String(testError));
      } finally {
        setBusyAction(null);
      }
    },
    [draft, load, savedDraft]
  );

  const dirty = useMemo(
    () => (draft ? isDraftDirty(savedDraft, draft) : false),
    [draft, savedDraft]
  );

  return {
    library,
    catalog,
    selection,
    editor,
    draft,
    preview,
    testResult,
    tokenState,
    loading,
    busyAction,
    error,
    dirty,
    load,
    openTemplate,
    openPreset,
    createPreset,
    forkTemplate,
    setDraft,
    runPreview,
    saveRevision,
    runTest,
  };
}
