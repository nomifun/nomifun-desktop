import { agentPlatform } from '@/common/adapter/ipcBridge';
import type {
  AgentCatalogResponse,
  AgentPresetDraft,
  AgentPresetDocument,
  AgentPresetEditorResponse,
  AgentPresetLibraryResponse,
  AgentPresetSummary,
  ChatRouteRecord,
  InstallationTokenStateResponse,
  OfficialPresetKey,
  OfficialPresetTemplate,
  ResolveAgentPresetPreviewResponse,
} from '@/common/types/agentPlatform';
import {
  AGENT_CHAT_MODEL_TASK,
  cloneDraft,
  isDraftDirty,
  runAgentPresetTest,
  type RunAgentPresetTestResult,
} from '@/common/types/agentPlatform';
import {
  agentUiErrorMessage,
  saveDraftRevisionWithPreview,
} from './model';
import { AGENT_PRESET_LIBRARY_SWR_KEY } from '@/renderer/hooks/agent/useAgentPresets';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { mutate } from 'swr';
import { useTranslation } from 'react-i18next';

type Selection =
  | { kind: 'template'; template: OfficialPresetTemplate }
  | { kind: 'preset'; preset: AgentPresetSummary }
  | null;

type BusyAction = 'preview' | 'save' | 'test' | 'fork' | 'create' | 'open' | 'delete' | null;

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
  const { t } = useTranslation();
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
  const [busyAction, setBusyAction] = useState<BusyAction>(null);
  const [openingPresetId, setOpeningPresetId] = useState<string | null>(null);
  const [deletingPresetId, setDeletingPresetId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const clearEditorState = useCallback(() => {
    setEditor(null);
    setDraftState(null);
    setSavedDraft(null);
    setPreview(null);
    setTestResult(null);
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [nextLibrary, capabilities, skills, mcpTools, nextTokenState] =
        await Promise.all([
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
        if (current?.kind === 'template') {
          const currentTemplate = nextLibrary.official_templates.find(
            (template) => template.template_key === current.template.template_key
          );
          if (currentTemplate) return { kind: 'template', template: currentTemplate };
        }
        if (current?.kind === 'preset') {
          const currentPreset = nextLibrary.user_presets.find(
            (preset) => preset.preset_id === current.preset.preset_id
          );
          if (currentPreset) return { kind: 'preset', preset: currentPreset };
        }
        const firstTemplate = nextLibrary.official_templates[0];
        return firstTemplate ? { kind: 'template', template: firstTemplate } : null;
      });
    } catch (loadError) {
      setError(agentUiErrorMessage(loadError, 'load'));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const refreshPresetLibraries = useCallback(
    async () => {
      await Promise.all([
        load(),
        mutate(AGENT_PRESET_LIBRARY_SWR_KEY),
      ]);
    },
    [load]
  );

  const openTemplate = useCallback((template: OfficialPresetTemplate) => {
    setSelection({ kind: 'template', template });
    clearEditorState();
    setError(null);
  }, [clearEditorState]);

  const applyEditor = useCallback((response: AgentPresetEditorResponse) => {
    const nextDraft = cloneDraft(response.draft);
    setEditor(response);
    setDraftState(nextDraft);
    setSavedDraft(response.revision ? cloneDraft(nextDraft) : null);
    setSelection({ kind: 'preset', preset: response.preset });
    setPreview(null);
    setTestResult(null);
  }, []);

  const openPreset = useCallback(
    async (preset: AgentPresetSummary) => {
      setBusyAction('open');
      setOpeningPresetId(preset.preset_id);
      setError(null);
      try {
        const response = await agentPlatform.getEditor.invoke({
          preset_id: preset.preset_id,
        });
        applyEditor(response);
      } catch (openError) {
        setError(agentUiErrorMessage(openError, 'open'));
      } finally {
        setOpeningPresetId(null);
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
        await refreshPresetLibraries();
        setSelection({ kind: 'preset', preset: response.preset });
      } catch (createError) {
        setError(agentUiErrorMessage(createError, 'create'));
      } finally {
        setBusyAction(null);
      }
    },
    [applyEditor, refreshPresetLibraries]
  );

  const forkTemplate = useCallback(
    async (
      templateKey: OfficialPresetKey,
      displayName: string,
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
            model_route_refs: modelRouteRefs,
            chat_route_records: chatRouteRecords,
          },
        });
        applyEditor(response);
        await refreshPresetLibraries();
        setSelection({ kind: 'preset', preset: response.preset });
      } catch (forkError) {
        setError(agentUiErrorMessage(forkError, 'fork'));
      } finally {
        setBusyAction(null);
      }
    },
    [applyEditor, refreshPresetLibraries]
  );

  const createConfiguredPreset = useCallback(async (displayName: string, document: AgentPresetDocument, description?: string) => {
    setBusyAction('create');
    setError(null);
    try {
      const response = await agentPlatform.createPreset.invoke({ display_name: displayName, description, document });
      applyEditor(response);
      await refreshPresetLibraries();
      setSelection({ kind: 'preset', preset: response.preset });
      return response;
    } catch (createError) {
      const missingModel = createError && typeof createError === 'object' && 'code' in createError && createError.code === 'MODEL_ROUTE_NOT_CONFIGURED';
      setError(missingModel ? t('agentSettings.workbench.modelNeeded') : agentUiErrorMessage(createError, 'create'));
      return null;
    } finally {
      setBusyAction(null);
    }
  }, [applyEditor, refreshPresetLibraries, t]);

  const discardChanges = useCallback(() => {
    if (savedDraft) setDraftState(cloneDraft(savedDraft));
    else if (editor) setDraftState(cloneDraft(editor.draft));
    setPreview(null);
    setTestResult(null);
  }, [savedDraft, editor]);

  const deletePreset = useCallback(
    async (preset: AgentPresetSummary) => {
      setBusyAction('delete');
      setDeletingPresetId(preset.preset_id);
      setError(null);
      try {
        await agentPlatform.deletePreset.invoke({ preset_id: preset.preset_id });

        if (
          selection?.kind === 'preset' &&
          selection.preset.preset_id === preset.preset_id
        ) {
          setSelection(null);
          clearEditorState();
        }
        await refreshPresetLibraries();
      } catch (deleteError) {
        setError(agentUiErrorMessage(deleteError, 'delete'));
      } finally {
        setDeletingPresetId(null);
        setBusyAction(null);
      }
    },
    [clearEditorState, refreshPresetLibraries, selection]
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
      const response = await resolveDraftPreview(draft);
      setPreview(response);
      return response;
    } catch (previewError) {
      setError(agentUiErrorMessage(previewError, 'preview'));
      return null;
    } finally {
      setBusyAction(null);
    }
  }, [draft, resolveDraftPreview]);

  const saveRevision = useCallback(async () => {
    if (!draft) return null;
    setBusyAction('save');
    setError(null);
    try {
      const result = await saveDraftRevisionWithPreview(draft, {
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
      await refreshPresetLibraries();
      setSelection({ kind: 'preset', preset: saved.preset });
      return saved;
    } catch (saveError) {
      setError(agentUiErrorMessage(saveError, 'save'));
      return null;
    } finally {
      setBusyAction(null);
    }
  }, [draft, refreshPresetLibraries, resolveDraftPreview]);

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
          await refreshPresetLibraries();
          setSelection({ kind: 'preset', preset: result.savedRevision.preset });
        }
      } catch (testError) {
        setError(agentUiErrorMessage(testError, 'test'));
      } finally {
        setBusyAction(null);
      }
    },
    [draft, refreshPresetLibraries, resolveDraftPreview, savedDraft]
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
    openingPresetId,
    deletingPresetId,
    error,
    dirty,
    load,
    openTemplate,
    openPreset,
    createPreset,
    createConfiguredPreset,
    discardChanges,
    forkTemplate,
    deletePreset,
    setDraft,
    runPreview,
    saveRevision,
    runTest,
  };
}
