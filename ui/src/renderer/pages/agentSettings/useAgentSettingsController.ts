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
import {
  agentUiErrorKey,
  agentUiErrorSubjects,
  isAgentModelConfigurationMissing,
  type AgentEditorReturn,
} from './model';
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
  modules: [],
  capabilities: [],
  skills: [],
  mcp_tools: [],
  roles: [],
};

export const upsertPresetSummary = (
  library: AgentPresetLibraryResponse,
  preset: AgentPresetSummary
): AgentPresetLibraryResponse => {
  const existingIndex = library.user_presets.findIndex(
    (candidate) => candidate.preset_id === preset.preset_id
  );
  const userPresets = existingIndex === -1
    ? [...library.user_presets, preset]
    : library.user_presets.map((candidate, index) =>
        index === existingIndex ? preset : candidate
      );

  return {
    ...library,
    user_presets: userPresets,
    fresh_start: {
      ...library.fresh_start,
      user_preset_count: userPresets.length,
    },
  };
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
  const [error, setErrorState] = useState<string | null>(null);
  const [errorSubjects, setErrorSubjects] = useState<string[]>([]);
  const [modelConfigurationMissing, setModelConfigurationMissing] = useState(false);
  const setError = useCallback((value: string | null) => {
    setErrorState(value);
    if (value === null) {
      setModelConfigurationMissing(false);
      setErrorSubjects([]);
    }
  }, []);
  const reportError = useCallback((value: unknown, operation: Parameters<typeof agentUiErrorKey>[1]) => {
    setError(t(agentUiErrorKey(value, operation)));
    setErrorSubjects(agentUiErrorSubjects(value));
  }, [setError, t]);

  const clearEditorState = useCallback(() => {
    setEditor(null);
    setDraftState(null);
    setSavedDraft(null);
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [nextLibrary, nextCatalog] =
        await Promise.all([
          agentPlatform.library.invoke(),
          agentPlatform.catalog.invoke(),
        ]);
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
      reportError(loadError, 'load');
    } finally {
      setLoading(false);
    }
  }, [mutate, reportError]);

  useEffect(() => {
    void load();
  }, [load]);

  const refreshPresetLibraries = useCallback(
    async () => {
      await load();
    },
    [load]
  );

  const publishPresetSummary = useCallback(async (preset: AgentPresetSummary) => {
    setLibrary((current) => current ? upsertPresetSummary(current, preset) : current);
    await mutate<AgentPresetLibraryResponse>(
      AGENT_PRESET_LIBRARY_SWR_KEY,
      (current) => current ? upsertPresetSummary(current, preset) : current,
      { revalidate: false }
    );
  }, [mutate]);

  const openTemplate = useCallback((template: OfficialPresetTemplate) => {
    setSelection({ kind: 'template', template });
    clearEditorState();
    setError(null);
  }, [clearEditorState]);

  const applyEditor = useCallback((response: AgentPresetEditorResponse) => {
    const nextDraft = cloneDraft(response.draft);
    setEditor({
      ...response,
      draft: nextDraft,
      revision: response.revision
        ? { ...response.revision, document: response.revision.document }
        : undefined,
    });
    setDraftState(nextDraft);
    setSavedDraft(response.revision ? cloneDraft(nextDraft) : null);
    setSelection({ kind: 'preset', preset: response.preset });
  }, []);

  const openPreset = useCallback(
    async (preset: AgentPresetSummary, returned?: Extract<AgentEditorReturn, { kind: 'preset' }>['draft']) => {
      setBusyAction('open');
      setOpeningPresetId(preset.preset_id);
      setError(null);
      try {
        const response = await agentPlatform.getEditor.invoke({
          preset_id: preset.preset_id,
        });
        applyEditor(response);
        if (returned?.preset_id === response.draft.preset_id &&
            JSON.stringify(returned.current_revision) === JSON.stringify(response.draft.current_revision)) {
          setDraftState({ ...response.draft, ...returned, document: {
            ...response.draft.document, ...returned.document,
            model_route_refs: response.draft.document.model_route_refs,
            chat_route_records: response.draft.document.chat_route_records,
          } });
        } else if (returned) {
          setError(t('agentSettings.workbench.returnChanged'));
        }
      } catch (openError) {
        reportError(openError, 'open');
      } finally {
        setOpeningPresetId(null);
        setBusyAction(null);
      }
    },
    [applyEditor, reportError, t]
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
        await publishPresetSummary(response.preset);
        await refreshPresetLibraries();
        await publishPresetSummary(response.preset);
        setSelection({ kind: 'preset', preset: response.preset });
      } catch (createError) {
        reportError(createError, 'create');
      } finally {
        setBusyAction(null);
      }
    },
    [applyEditor, publishPresetSummary, refreshPresetLibraries, reportError]
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
        await publishPresetSummary(response.preset);
        await refreshPresetLibraries();
        await publishPresetSummary(response.preset);
        setSelection({ kind: 'preset', preset: response.preset });
      } catch (forkError) {
        reportError(forkError, 'fork');
      } finally {
        setBusyAction(null);
      }
    },
    [applyEditor, publishPresetSummary, refreshPresetLibraries, reportError]
  );

  const createConfiguredPreset = useCallback(async (displayName: string, document: AgentPresetDocument, description?: string) => {
    setBusyAction('create');
    setError(null);
    try {
      const response = await agentPlatform.createPreset.invoke({
        display_name: displayName,
        description,
        document,
      });
      applyEditor(response);
      await publishPresetSummary(response.preset);
      await refreshPresetLibraries();
      await publishPresetSummary(response.preset);
      setSelection({ kind: 'preset', preset: response.preset });
      return response;
    } catch (createError) {
      const missingModel = isAgentModelConfigurationMissing(createError);
      if (missingModel) setError(t('agentSettings.workbench.modelNeeded'));
      else reportError(createError, 'create');
      setModelConfigurationMissing(missingModel);
      return null;
    } finally {
      setBusyAction(null);
    }
  }, [applyEditor, publishPresetSummary, refreshPresetLibraries, reportError, t]);

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
        reportError(deleteError, 'delete');
      } finally {
        setDeletingPresetId(null);
        setBusyAction(null);
      }
    },
    [clearEditorState, refreshPresetLibraries, reportError, selection]
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
        document: draft.document,
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
      await publishPresetSummary(saved.preset);
      await refreshPresetLibraries();
      await publishPresetSummary(saved.preset);
      setSelection({ kind: 'preset', preset: saved.preset });
      return saved;
    } catch (saveError) {
      const missingModel = isAgentModelConfigurationMissing(saveError);
      if (missingModel) setError(t('agentSettings.workbench.modelNeeded'));
      else reportError(saveError, 'save');
      setModelConfigurationMissing(missingModel);
      return null;
    } finally {
      setBusyAction(null);
    }
  }, [draft, publishPresetSummary, refreshPresetLibraries, reportError, t]);

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
    errorSubjects,
    modelConfigurationMissing,
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
