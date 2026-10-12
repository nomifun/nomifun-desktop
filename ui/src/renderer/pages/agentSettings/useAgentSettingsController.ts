import { agentPlatform } from '@/common/adapter/ipcBridge';
import type {
  AgentCatalogResponse,
  AgentPresetDraft,
  AgentPresetDocument,
  AgentPresetEditorResponse,
  AgentPresetLibraryResponse,
  AgentPresetRecommendedCapabilitiesResponse,
  AgentPresetSummary,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import {
  cloneAgentPresetDocument,
  cloneDraft,
  isDraftDirty,
} from '@/common/types/agentPlatform';
import {
  agentUiErrorKey,
  agentUiErrorSubjects,
  isAgentModelConfigurationMissing,
  TEMPLATE_I18N_PATH,
  type AgentEditorReturn,
} from './model';
import { AGENT_PRESET_LIBRARY_SWR_KEY } from '@/renderer/hooks/agent/useAgentPresets';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useSWRConfig } from 'swr';
import { useTranslation } from 'react-i18next';
import { useAuth } from '@/renderer/hooks/context/AuthContext';
import { AgentEditorDrafts } from './agentEditorDrafts';

type Selection =
  | { kind: 'template'; template: OfficialPresetTemplate }
  | { kind: 'preset'; preset: AgentPresetSummary }
  | null;

type BusyAction = 'save' | 'create' | 'open' | 'delete' | 'recommend' | null;

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
  if (preset.source === 'official') {
    return {
      ...library,
      official_presets: Object.fromEntries(
        Object.entries(library.official_presets).map(([key, current]) =>
          [key, current.preset_id === preset.preset_id ? preset : current]
        )
      ),
    };
  }
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
  const { user } = useAuth();
  const editingDrafts = useMemo(() => user ? new AgentEditorDrafts(user.id) : null, [user?.id]);
  const { mutate } = useSWRConfig();
  const [library, setLibrary] = useState<AgentPresetLibraryResponse | null>(null);
  const [catalog, setCatalog] = useState<AgentCatalogResponse>(emptyCatalog);
  const [selection, setSelection] = useState<Selection>(null);
  const [editor, setEditor] = useState<AgentPresetEditorResponse | null>(null);
  const [draft, setDraftState] = useState<AgentPresetDraft | null>(null);
  const [savedDraft, setSavedDraft] = useState<AgentPresetDraft | null>(null);
  const [recommendation, setRecommendation] = useState<AgentPresetRecommendedCapabilitiesResponse | null>(null);
  const editorGenerationRef = useRef(0);
  const recommendingRef = useRef(false);
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
    editorGenerationRef.current += 1;
    setEditor(null);
    setDraftState(null);
    setSavedDraft(null);
    setRecommendation(null);
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
          const currentPreset = [...nextLibrary.user_presets, ...Object.values(nextLibrary.official_presets)].find(
            (preset) => preset.preset_id === current.preset.preset_id
          );
          if (currentPreset) return { kind: 'preset', preset: currentPreset };
        }
        const remembered = editingDrafts?.readSelection();
        const firstTemplate = (remembered?.kind === 'template'
          ? nextLibrary.official_templates.find(template => template.template_key === remembered.template_key)
          : undefined) ?? nextLibrary.official_templates[0];
        return firstTemplate ? { kind: 'template', template: firstTemplate } : null;
      });
    } catch (loadError) {
      reportError(loadError, 'load');
    } finally {
      setLoading(false);
    }
  }, [mutate, reportError, editingDrafts]);

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

  const applyEditor = useCallback((response: AgentPresetEditorResponse) => {
    editorGenerationRef.current += 1;
    setRecommendation(null);
    editingDrafts?.rememberSelection({ kind: 'preset', preset_id: response.preset.preset_id });
    const nextDraft = cloneDraft(response.draft);
    if (response.preset.source === 'official' && nextDraft.source_template_key &&
        (!response.revision || nextDraft.display_name === nextDraft.source_template_key)) {
      const path = TEMPLATE_I18N_PATH[nextDraft.source_template_key];
      nextDraft.display_name = t(`agentSettings.template.${path}.name`);
      nextDraft.description ??= t(`agentSettings.template.${path}.description`);
    }
    setEditor({
      ...response,
      draft: nextDraft,
      revision: response.revision
        ? { ...response.revision, document: cloneAgentPresetDocument(response.revision.document) }
        : undefined,
    });
    setDraftState(nextDraft);
    setSavedDraft(response.revision || response.preset.source === 'official' ? cloneDraft(nextDraft) : null);
    setSelection({ kind: 'preset', preset: response.preset });
    return nextDraft;
  }, [editingDrafts, t]);

  const openPreset = useCallback(
    async (preset: AgentPresetSummary, returned?: Extract<AgentEditorReturn, { kind: 'preset' }>['draft']) => {
      setBusyAction('open');
      setOpeningPresetId(preset.preset_id);
      setError(null);
      try {
        const response = await agentPlatform.getEditor.invoke({
          preset_id: preset.preset_id,
        });
        const responseDraft = applyEditor(response);
        returned ??= editingDrafts?.readPreset(preset.preset_id);
        if (returned?.preset_id === response.draft.preset_id &&
            JSON.stringify(returned.current_revision) === JSON.stringify(response.draft.current_revision)) {
          setDraftState(cloneDraft({ ...responseDraft, ...returned, document: {
            ...responseDraft.document, ...returned.document,
            model_route_refs: responseDraft.document.model_route_refs,
            chat_route_records: responseDraft.document.chat_route_records,
          } }));
        } else if (returned) {
          editingDrafts?.removePreset(preset.preset_id);
          setError(t('agentSettings.workbench.returnChanged'));
        }
      } catch (openError) {
        reportError(openError, 'open');
      } finally {
        setOpeningPresetId(null);
        setBusyAction(null);
      }
    },
    [applyEditor, reportError, t, editingDrafts]
  );

  const openTemplate = useCallback(async (template: OfficialPresetTemplate) => {
    const preset = library?.official_presets[template.template_key];
    if (!preset) {
      reportError({ code: 'PRESET_NOT_FOUND' }, 'open');
      return;
    }
    await openPreset(preset);
  }, [library, openPreset, reportError]);

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
    setRecommendation(null);
    if (draft) editingDrafts?.removePreset(draft.preset_id);
    if (savedDraft) setDraftState(cloneDraft(savedDraft));
    else if (editor) setDraftState(cloneDraft(editor.draft));
  }, [savedDraft, editor, draft?.preset_id, editingDrafts]);

  const deletePreset = useCallback(
    async (preset: AgentPresetSummary) => {
      setBusyAction('delete');
      setDeletingPresetId(preset.preset_id);
      setError(null);
      try {
        await agentPlatform.deletePreset.invoke({ preset_id: preset.preset_id });
        editingDrafts?.removePreset(preset.preset_id);

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
    [clearEditorState, refreshPresetLibraries, reportError, selection, editingDrafts]
  );

  const setDraft = useCallback((next: AgentPresetDraft) => {
    setRecommendation(null);
    editingDrafts?.writePreset(next);
    setDraftState(next);
  }, [editingDrafts]);

  const saveRevision = useCallback(async (draftToSave = draft) => {
    if (!draftToSave) return null;
    setBusyAction('save');
    setError(null);
    try {
      const saved = await agentPlatform.saveRevision.invoke({
        preset_id: draftToSave.preset_id,
        request: {
          expected_current_revision: draftToSave.current_revision,
          draft: draftToSave,
        },
      });
      editingDrafts?.removePreset(draftToSave.preset_id);
      const nextDraft: AgentPresetDraft = {
        ...draftToSave,
        current_revision: saved.revision.reference,
      };
      setDraftState(nextDraft);
      setSavedDraft(cloneDraft(nextDraft));
      setRecommendation(null);
      setEditor((current) =>
        current
          ? {
              preset: saved.preset,
              revision: {
                ...saved.revision,
                document: cloneAgentPresetDocument(saved.revision.document),
              },
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
  }, [draft, publishPresetSummary, refreshPresetLibraries, reportError, t, editingDrafts]);

  const dirty = useMemo(
    () => (draft ? isDraftDirty(savedDraft, draft) : false),
    [draft, savedDraft]
  );

  const previewRecommendedCapabilities = useCallback(async () => {
    if (!draft?.source_template_key || editor?.preset.source !== 'official' || dirty || busyAction || recommendingRef.current) return;
    recommendingRef.current = true;
    const generation = editorGenerationRef.current;
    setBusyAction('recommend');
    setError(null);
    setRecommendation(null);
    try {
      const preview = await agentPlatform.recommendedCapabilities.invoke({ preset_id: draft.preset_id });
      if (generation !== editorGenerationRef.current) return;
      // Do not adopt another editor's metadata without refreshing our saved baseline.
      if (preview.draft.preset_id !== draft.preset_id ||
          preview.draft.current_revision?.preset_id !== draft.current_revision?.preset_id ||
          preview.draft.current_revision?.revision !== draft.current_revision?.revision ||
          preview.draft.current_revision?.revision_digest !== draft.current_revision?.revision_digest) {
        reportError({ status: 409 }, 'open');
        return;
      }
      setRecommendation(preview);
    } catch (previewError) {
      if (generation === editorGenerationRef.current) reportError(previewError, 'open');
    } finally {
      recommendingRef.current = false;
      if (generation === editorGenerationRef.current) setBusyAction(null);
    }
  }, [draft, editor?.preset.source, dirty, busyAction, setError, reportError]);

  const applyRecommendedCapabilities = useCallback(async () => {
    if (!recommendation || recommendation.draft.preset_id !== draft?.preset_id || dirty || busyAction || recommendingRef.current) return null;
    recommendingRef.current = true;
    try {
      return await saveRevision(recommendation.draft);
    } finally {
      recommendingRef.current = false;
    }
  }, [recommendation, draft?.preset_id, dirty, busyAction, saveRevision]);

  return {
    editingDrafts,
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
    recommendation,
    previewRecommendedCapabilities,
    applyRecommendedCapabilities,
    dismissRecommendation: () => setRecommendation(null),
    load,
    openTemplate,
    openPreset,
    createPreset,
    createConfiguredPreset,
    discardChanges,
    deletePreset,
    setDraft,
    saveRevision,
  };
}
