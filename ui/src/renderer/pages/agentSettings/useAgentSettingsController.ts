import { agentPlatform } from '@/common/adapter/ipcBridge';
import type {
  AgentCatalogResponse,
  AgentPresetDraft,
  AgentPresetDocument,
  AgentPresetEditorResponse,
  AgentPresetLibraryResponse,
  AgentPresetSummary,
  ChatRouteRecord,
  OfficialPresetKey,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import {
  AGENT_CHAT_MODEL_TASK,
  cloneDraft,
  isDraftDirty,
} from '@/common/types/agentPlatform';
import { agentUiErrorMessage } from './model';
import { AGENT_PRESET_LIBRARY_SWR_KEY } from '@/renderer/hooks/agent/useAgentPresets';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useSWRConfig } from 'swr';
import { useTranslation } from 'react-i18next';

type Selection =
  | { kind: 'template'; template: OfficialPresetTemplate }
  | { kind: 'preset'; preset: AgentPresetSummary }
  | null;

type BusyAction = 'save' | 'fork' | 'create' | 'open' | 'delete' | null;

const emptyCatalog: AgentCatalogResponse = {
  capabilities: [],
  skills: [],
  mcp_tools: [],
};

export function useAgentSettingsController() {
  const { t } = useTranslation();
  const { mutate } = useSWRConfig();
  const [library, setLibrary] = useState<AgentPresetLibraryResponse | null>(null);
  const [catalog, setCatalog] = useState<AgentCatalogResponse>(emptyCatalog);
  const [selection, setSelection] = useState<Selection>(null);
  const [editor, setEditor] = useState<AgentPresetEditorResponse | null>(null);
  const [draft, setDraftState] = useState<AgentPresetDraft | null>(null);
  const [savedDraft, setSavedDraft] = useState<AgentPresetDraft | null>(null);
  const [loading, setLoading] = useState(true);
  const [busyAction, setBusyAction] = useState<BusyAction>(null);
  const [openingPresetId, setOpeningPresetId] = useState<string | null>(null);
  const [deletingPresetId, setDeletingPresetId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const clearEditorState = useCallback(() => {
    setEditor(null);
    setDraftState(null);
    setSavedDraft(null);
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [nextLibrary, capabilities, skills, mcpTools] =
        await Promise.all([
          agentPlatform.library.invoke(),
          agentPlatform.capabilities.invoke(),
          agentPlatform.skills.invoke(),
          agentPlatform.mcpTools.invoke(),
        ]);
      const nextCatalog = { capabilities, skills, mcp_tools: mcpTools };
      setLibrary(nextLibrary);
      // A selector on another route may be unmounted. Revalidation alone
      // does not refresh that cache, so publish this authoritative response
      // before enabling "Use Agent" and navigating to the new session.
      await mutate(AGENT_PRESET_LIBRARY_SWR_KEY, nextLibrary, { revalidate: false });
      setCatalog(nextCatalog);
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
  }, [mutate]);

  useEffect(() => {
    void load();
  }, [load]);

  const refreshPresetLibraries = useCallback(
    async () => {
      await load();
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
            reuse_existing: false,
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
  }, []);

  const saveRevision = useCallback(async () => {
    if (!draft) return null;
    setBusyAction('save');
    setError(null);
    try {
      const saved = await agentPlatform.saveRevision.invoke({
        preset_id: draft.preset_id,
        request: {
          expected_current_revision: draft.current_revision,
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
      await refreshPresetLibraries();
      setSelection({ kind: 'preset', preset: saved.preset });
      return saved;
    } catch (saveError) {
      setError(agentUiErrorMessage(saveError, 'save'));
      return null;
    } finally {
      setBusyAction(null);
    }
  }, [draft, refreshPresetLibraries]);

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
    saveRevision,
  };
}
