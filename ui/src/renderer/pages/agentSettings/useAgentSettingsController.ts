import { agentPlatform, application, mcpService } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import type {
  AgentCatalogResponse,
  AgentPresetDraft,
  AgentPresetEditorResponse,
  AgentPresetLibraryResponse,
  AgentPresetSummary,
  ChatRouteRecord,
  InstallationTokenStateResponse,
  OfficialPresetKey,
  OfficialPresetTemplate,
  ResolveAgentPresetPreviewResponse,
  TemplateResourceSelection,
} from '@/common/types/agentPlatform';
import {
  AGENT_CHAT_MODEL_TASK,
  cloneDraft,
  isDraftDirty,
  runAgentPresetTest,
  type RunAgentPresetTestResult,
} from '@/common/types/agentPlatform';
import { saveDraftRevisionWithPreview, withHostResolvedWorkspaceBinding } from './model';
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
  const [hostWorkDir, setHostWorkDir] = useState<string | null>(null);
  const [connectors, setConnectors] = useState<IMcpServer[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyAction, setBusyAction] = useState<
    'preview' | 'save' | 'test' | 'fork' | 'create' | null
  >(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [
        nextLibrary,
        capabilities,
        skills,
        mcpTools,
        nextTokenState,
        systemInfo,
        nextConnectors,
      ] = await Promise.all([
        agentPlatform.library.invoke(),
        agentPlatform.capabilities.invoke(),
        agentPlatform.skills.invoke(),
        agentPlatform.mcpTools.invoke(),
        agentPlatform.installationToken.status.invoke().catch(() => null),
        application.systemInfo.invoke().catch(() => null),
        mcpService.listServers.invoke().catch(() => []),
      ]);
      const nextCatalog = { capabilities, skills, mcp_tools: mcpTools };
      setLibrary(nextLibrary);
      setCatalog(nextCatalog);
      setTokenState(nextTokenState);
      setConnectors(nextConnectors);
      setHostWorkDir(systemInfo?.workDir?.trim() ? systemInfo.workDir.trim() : null);
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

  const applyEditor = useCallback(
    (response: AgentPresetEditorResponse) => {
      const nextDraft = withHostResolvedWorkspaceBinding(cloneDraft(response.draft), hostWorkDir);
      setEditor(response);
      setDraftState(nextDraft);
      setSavedDraft(response.revision ? cloneDraft(nextDraft) : null);
      setSelection({ kind: 'preset', preset: response.preset });
      setPreview(null);
      setTestResult(null);
    },
    [hostWorkDir]
  );

  const openPreset = useCallback(
    async (preset: AgentPresetSummary) => {
      setSelection({ kind: 'preset', preset });
      setBusyAction('create');
      setError(null);
      try {
        const response = await agentPlatform.getEditor.invoke({
          preset_id: preset.preset_id,
        });
        applyEditor(response);
      } catch (openError) {
        setError(String(openError));
      } finally {
        setBusyAction(null);
      }
    },
    [applyEditor]
  );

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
      modelRouteRefs: Record<string, string>,
      chatRouteRecords: Partial<Record<typeof AGENT_CHAT_MODEL_TASK, ChatRouteRecord>>
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
            chat_route_records: chatRouteRecords,
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

  const resolveDraftPreview = useCallback(
    (nextDraft: AgentPresetDraft) =>
      agentPlatform.resolvePreview.invoke({
        preset_id: nextDraft.preset_id,
        request: previewRequest(nextDraft),
      }),
    []
  );

  const runPreview = useCallback(async (): Promise<ResolveAgentPresetPreviewResponse | null> => {
    if (!draft) return null;
    setBusyAction('preview');
    setError(null);
    try {
      const resolvedDraft = withHostResolvedWorkspaceBinding(draft, hostWorkDir);
      if (resolvedDraft !== draft) setDraftState(resolvedDraft);
      const response = await resolveDraftPreview(resolvedDraft);
      setPreview(response);
      return response;
    } catch (previewError) {
      setError(String(previewError));
      return null;
    } finally {
      setBusyAction(null);
    }
  }, [draft, hostWorkDir, resolveDraftPreview]);

  const saveRevision = useCallback(async () => {
    if (!draft) return null;
    setBusyAction('save');
    setError(null);
    try {
      const resolvedDraft = withHostResolvedWorkspaceBinding(draft, hostWorkDir);
      if (resolvedDraft !== draft) setDraftState(resolvedDraft);
      const result = await saveDraftRevisionWithPreview(resolvedDraft, {
        preview: resolveDraftPreview,
        save: async (nextDraft, freshPreview) =>
          agentPlatform.saveRevision.invoke({
            preset_id: nextDraft.preset_id,
            request: {
              expected_current_revision: nextDraft.current_revision,
              preview_digest: freshPreview.preview_digest,
              draft: nextDraft,
            },
          }),
      });
      setPreview(result.preview);
      if (!result.saved) return null;
      const saved = result.saved;
      const nextDraft: AgentPresetDraft = {
        ...resolvedDraft,
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
  }, [draft, hostWorkDir, load, resolveDraftPreview]);

  const runTest = useCallback(
    async (input: string) => {
      if (!draft) return;
      setBusyAction('test');
      setError(null);
      try {
        const resolvedDraft = withHostResolvedWorkspaceBinding(draft, hostWorkDir);
        if (resolvedDraft !== draft) setDraftState(resolvedDraft);
        const dirty = isDraftDirty(savedDraft, resolvedDraft);
        const result = await runAgentPresetTest({
          draft: resolvedDraft,
          dirty,
          input,
          idempotencyKey: idempotencyKey(),
          ports: {
            preview: async (nextDraft) => resolveDraftPreview(nextDraft),
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
            ...resolvedDraft,
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
    [draft, hostWorkDir, load, resolveDraftPreview, savedDraft]
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
    hostWorkDir,
    connectors,
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
