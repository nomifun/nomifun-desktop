/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import type {
  ConfigurePluginRequest,
  ApplyPluginSourceEditRequest,
  PluginDetail,
  PluginLibraryResponse,
  PluginMountId,
  PluginProjectDetail,
  PluginProjectId,
  GeneratedPluginDraft,
  ImportPluginRequest,
  SharePluginRequest,
  UpdatePluginDependenciesRequest,
} from '@/common/types/pluginPlatform';
import { Alert, Button, Modal } from '@arco-design/web-react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useSearchParams } from 'react-router-dom';
import HubPageShell from '@/renderer/components/layout/HubPageShell';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { useGuidModelSelection } from '@/renderer/pages/guid/hooks/useGuidModelSelection';
import type { PluginMountBusyAction } from './PluginLibraryView';
import PluginConfigurationDialog from './PluginConfigurationDialog';
import type { PluginProjectBusyAction } from './PluginWorkshopView';
import { pluginRuntimeSetEnabledRequest } from './runtime/model';
import { pluginRuntimeProduct, type PluginRuntimeDraft, type PluginRuntimeWorkspace } from '@/common/adapter/pluginRuntimeProductBridge';
import { emptyItem, updatePluginRuntimeWorkspace, MINIAPP_LIBRARY_CHANGED, libraryChanged } from './runtime/libraryState';
import {
  PluginCandidateApplyModal,
  PluginCandidateTestModal,
} from './PluginWorkbenchDialogs';
import PluginSourceEditDialog from './PluginSourceEditDialog';
import PluginDependencyDialog from './PluginDependencyDialog';
import PluginShareExportDialog from './PluginShareExportDialog';
import PluginProductHome from './PluginProductHome';
import PluginCreatorSurface, {
  type PluginAiBusyStep,
  type PluginCreatorMessage,
} from './PluginCreatorSurface';
import PluginProductDetail from './PluginProductDetail';
import PluginSmartImportDialog from './PluginSmartImportDialog';
import {
  pluginDraftSourceEdits,
  pluginPackageId,
  pluginProductItems,
  pluginProductMatches,
  type PluginProductItem,
} from './pluginProductModel';
import {
  applyPluginCandidateRequest,
  buildPluginProjectRequest,
  deletePluginDataRequest,
  pluginLoadFailure,
  restorePluginRequest,
  retryPluginRequest,
  setPluginEnabledRequest,
  testPluginCandidateRequest,
  uninstallPluginRequest,
} from './pluginWorkbenchModel';
import type {
  PluginApplyTargetSelection,
  PluginLoadFailure,
} from './pluginWorkbenchModel';
import { PluginStatePanel } from './PluginWorkbenchState';
import styles from './PluginWorkbenchPage.module.css';

type WorkbenchTab = 'library' | 'workshop';
type PluginProductSurface = 'home' | 'creator' | 'detail';
type PluginAgentUsage = { presetId: string; displayName: string; capabilityCount: number };

const PluginWorkbenchPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { current_model } = useGuidModelSelection('nomi');
  const [message, messageContext] = useArcoMessage({ maxCount: 8 });
  const [searchParams, setSearchParams] = useSearchParams();
  const activeTab: WorkbenchTab =
    searchParams.get('tab') === 'workshop' ? 'workshop' : 'library';
  const [library, setLibrary] = useState<PluginLibraryResponse | null>(null);
  const [runtimeDrafts, setRuntimeDrafts] = useState<PluginRuntimeDraft[]>([]);
  const [workspace, setWorkspace] = useState<PluginRuntimeWorkspace>({revision: 0, collections: [], items: {}});
  const [loading, setLoading] = useState(true);
  const [failure, setFailure] = useState<PluginLoadFailure | null>(null);
  const [selectedMountId, setSelectedMountId] = useState<PluginMountId | null>(null);
  const [selectedProjectId, setSelectedProjectId] = useState<PluginProjectId | null>(null);
  const [mountDetail, setMountDetail] = useState<PluginDetail | null>(null);
  const [projectDetail, setProjectDetail] = useState<PluginProjectDetail | null>(null);
  const [mountDetailLoading, setMountDetailLoading] = useState(false);
  const [projectDetailLoading, setProjectDetailLoading] = useState(false);
  const [mountFailure, setMountFailure] = useState<PluginLoadFailure | null>(null);
  const [projectFailure, setProjectFailure] = useState<PluginLoadFailure | null>(null);
  const [mutationFailure, setMutationFailure] = useState<PluginLoadFailure | null>(null);
  const [projectMutationFailure, setProjectMutationFailure] =
    useState<PluginLoadFailure | null>(null);
  const [busyAction, setBusyAction] = useState<PluginMountBusyAction>(null);
  const [projectBusyAction, setProjectBusyAction] =
    useState<PluginProjectBusyAction>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [importDialogVisible, setImportDialogVisible] = useState(false);
  const [testDialogVisible, setTestDialogVisible] = useState(false);
  const [applyDialogVisible, setApplyDialogVisible] = useState(false);
  const [sourceEditDialogVisible, setSourceEditDialogVisible] = useState(false);
  const [dependencyDialogVisible, setDependencyDialogVisible] = useState(false);
  const [shareDialogVisible, setShareDialogVisible] = useState(false);
  const [configureDialogVisible, setConfigureDialogVisible] = useState(false);
  const [surface, setSurface] = useState<PluginProductSurface>('home');
  const [aiDraft, setAiDraft] = useState<GeneratedPluginDraft | null>(null);
  const [aiBusyStep, setAiBusyStep] = useState<PluginAiBusyStep>(null);
  const [creatorMessages, setCreatorMessages] = useState<PluginCreatorMessage[]>([]);
  const [homeBusyMountId, setHomeBusyMountId] = useState<string | null>(null);
  const [agentUsage, setAgentUsage] = useState<PluginAgentUsage[]>([]);
  const mountLoadSequence = useRef(0);
  const projectLoadSequence = useRef(0);

  const setActiveTab = useCallback(
    (tab: WorkbenchTab) => {
      const next = new URLSearchParams(searchParams);
      if (tab === 'library') next.delete('tab');
      else next.set('tab', tab);
      setSearchParams(next, { replace: true });
    },
    [searchParams, setSearchParams]
  );

  const refreshLibrary = useCallback(async () => {
    setLoading(true);
    try {
      const [next, drafts, organization] = await Promise.all([ipcBridge.plugins.list.invoke(), pluginRuntimeProduct.drafts.invoke(), pluginRuntimeProduct.workspace.invoke()]);
      setLibrary(next);
      setRuntimeDrafts(drafts);
      setWorkspace(organization);
      setFailure(null);
      setSelectedMountId((current) => {
        if (current && next.plugins.some((plugin) => plugin.mount_id === current)) {
          return current;
        }
        return next.plugins[0]?.mount_id ?? null;
      });
      setSelectedProjectId((current) => {
        if (current && next.projects.some((project) => project.project_id === current)) {
          return current;
        }
        return next.projects[0]?.project_id ?? null;
      });
    } catch (error) {
      console.error('[plugins] failed to load library', error);
      setFailure(pluginLoadFailure(error, 'platform'));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshLibrary();
    window.addEventListener(MINIAPP_LIBRARY_CHANGED, refreshLibrary);
    return () => window.removeEventListener(MINIAPP_LIBRARY_CHANGED, refreshLibrary);
  }, [refreshLibrary]);

  const linkedPluginId = searchParams.get('plugin');
  useEffect(() => {
    const mount = library?.plugins.find((item) => item.mount_id === linkedPluginId);
    if (mount) { setSelectedMountId(mount.mount_id); setSurface('detail'); }
  }, [linkedPluginId, library]);

  const loadMountDetail = useCallback(async (mountId: PluginMountId) => {
    const sequence = ++mountLoadSequence.current;
    setMountDetailLoading(true);
    setMountFailure(null);
    try {
      const next = await ipcBridge.plugins.getMount.invoke({ mount_id: mountId });
      if (sequence !== mountLoadSequence.current) return;
      setMountDetail(next);
    } catch (error) {
      if (sequence !== mountLoadSequence.current) return;
      console.error('[plugins] failed to load Mount detail', error);
      setMountDetail(null);
      setMountFailure(pluginLoadFailure(error, 'resource'));
    } finally {
      if (sequence === mountLoadSequence.current) setMountDetailLoading(false);
    }
  }, []);

  const loadProjectDetail = useCallback(async (projectId: PluginProjectId) => {
    const sequence = ++projectLoadSequence.current;
    setProjectDetailLoading(true);
    setProjectFailure(null);
    try {
      const next = await ipcBridge.plugins.getProject.invoke({ project_id: projectId });
      if (sequence !== projectLoadSequence.current) return;
      setProjectDetail(next);
    } catch (error) {
      if (sequence !== projectLoadSequence.current) return;
      console.error('[plugins] failed to load Project detail', error);
      setProjectDetail(null);
      setProjectFailure(pluginLoadFailure(error, 'resource'));
    } finally {
      if (sequence === projectLoadSequence.current) setProjectDetailLoading(false);
    }
  }, []);

  useEffect(() => {
    setMutationFailure(null);
    if (activeTab !== 'library' || !selectedMountId) {
      mountLoadSequence.current += 1;
      setMountDetail(null);
      setMountDetailLoading(false);
      setMountFailure(null);
      return;
    }
    void loadMountDetail(selectedMountId);
  }, [activeTab, loadMountDetail, selectedMountId]);

  useEffect(() => {
    if (activeTab !== 'workshop' || !selectedProjectId) {
      projectLoadSequence.current += 1;
      setProjectDetail(null);
      setProjectDetailLoading(false);
      setProjectFailure(null);
      return;
    }
    void loadProjectDetail(selectedProjectId);
  }, [activeTab, loadProjectDetail, selectedProjectId]);

  useEffect(() => {
    let canceled = false;
    const loadAgentUsage = async () => {
      if (!mountDetail?.capabilities.length) {
        setAgentUsage([]);
        return;
      }
      const capabilityKeys = new Set(
        mountDetail.capabilities.map((capability) => `${capability.capability_id}@${capability.capability_version}`)
      );
      try {
        const library = await agentPlatform.library.invoke();
        const editors = await Promise.all(
          library.user_presets.map(async (preset) => ({
            preset,
            editor: await agentPlatform.getEditor.invoke({ preset_id: preset.preset_id }),
          }))
        );
        if (canceled) return;
        setAgentUsage(editors.flatMap(({ preset, editor }) => {
          const selected = [
            ...editor.draft.document.initial_capabilities,
            ...editor.draft.document.on_demand_capabilities,
          ].filter((selection) =>
            capabilityKeys.has(`${selection.capability.id}@${selection.capability.version}`)
          );
          return selected.length ? [{
            presetId: preset.preset_id,
            displayName: preset.display_name,
            capabilityCount: selected.length,
          }] : [];
        }));
      } catch (error) {
        if (!canceled) {
          console.warn('[plugins] failed to resolve Agent usage', error);
          setAgentUsage([]);
        }
      }
    };
    void loadAgentUsage();
    return () => { canceled = true; };
  }, [mountDetail?.capabilities]);

  const refreshVisible = useCallback(async () => {
    const tasks: Promise<unknown>[] = [refreshLibrary()];
    if (activeTab === 'library' && selectedMountId) {
      tasks.push(loadMountDetail(selectedMountId));
    }
    if (activeTab === 'workshop' && selectedProjectId) {
      tasks.push(loadProjectDetail(selectedProjectId));
    }
    await Promise.all(tasks);
  }, [
    activeTab,
    loadMountDetail,
    loadProjectDetail,
    refreshLibrary,
    selectedMountId,
    selectedProjectId,
  ]);

  const runMountMutation = useCallback(
    async (
      action: Exclude<PluginMountBusyAction, null>,
      invoke: (detail: PluginDetail) => Promise<PluginDetail>,
      successKey: string
    ) => {
      if (!mountDetail || busyAction) return;
      setBusyAction(action);
      setMutationFailure(null);
      try {
        const next = await invoke(mountDetail);
        setMountDetail(next);
        message.success(t(successKey));
        await refreshLibrary();
      } catch (error) {
        console.error(`[plugins] Mount action ${action} failed`, error);
        setMutationFailure(pluginLoadFailure(error, 'resource'));
      } finally {
        setBusyAction(null);
      }
    },
    [busyAction, mountDetail, message, refreshLibrary, t]
  );

  const confirmMountMutation = useCallback(
    (
      action: Exclude<PluginMountBusyAction, null>,
      title: string,
      content: string,
      invoke: (detail: PluginDetail) => Promise<PluginDetail>,
      successKey: string
    ) => {
      Modal.confirm({
        title,
        content,
        okText: t('pluginWorkbench.actions.confirm'),
        cancelText: t('pluginWorkbench.actions.cancel'),
        okButtonProps: { status: 'danger' },
        onOk: () => runMountMutation(action, invoke, successKey),
      });
    },
    [runMountMutation, t]
  );

  const handleUninstall = useCallback(() => {
    if (!mountDetail) return;
    confirmMountMutation(
      'uninstall',
      t('pluginWorkbench.confirm.uninstallTitle'),
      t('pluginWorkbench.confirm.uninstallBody'),
      (detail) => ipcBridge.plugins.uninstall.invoke(uninstallPluginRequest(detail)),
      'pluginWorkbench.messages.uninstalled'
    );
  }, [confirmMountMutation, mountDetail, t]);

  const handleDeleteData = useCallback(() => {
    if (!mountDetail) return;
    Modal.confirm({
      title: t('pluginWorkbench.confirm.deleteDataTitle'),
      content: t('pluginWorkbench.confirm.deleteDataBody'),
      okText: t('pluginWorkbench.actions.deleteData'),
      cancelText: t('pluginWorkbench.actions.cancel'),
      okButtonProps: { status: 'danger' },
      onOk: async () => {
        setBusyAction('delete_data');
        setMutationFailure(null);
        try {
          await ipcBridge.plugins.deleteData.invoke(deletePluginDataRequest(mountDetail));
          message.success(t('pluginWorkbench.messages.dataDeleted'));
          setMountDetail(null);
          await refreshLibrary();
        } catch (error) {
          console.error('[plugins] deleting retained Mount data failed', error);
          setMutationFailure(pluginLoadFailure(error, 'resource'));
        } finally {
          setBusyAction(null);
        }
      },
    });
  }, [mountDetail, message, refreshLibrary, t]);

  const handleConfigure = useCallback(
    async (request: ConfigurePluginRequest) => {
      if (!mountDetail || busyAction) return;
      setBusyAction('configure');
      setMutationFailure(null);
      try {
        const next = await ipcBridge.plugins.configure.invoke(request);
        setMountDetail(next);
        setConfigureDialogVisible(false);
        message.success(t('pluginWorkbench.messages.configured'));
        await Promise.all([
          refreshLibrary(),
          loadMountDetail(next.summary.mount_id),
        ]);
      } catch (error) {
        console.error('[plugins] configuring Mount failed', error);
        setMutationFailure(pluginLoadFailure(error, 'resource'));
      } finally {
        setBusyAction(null);
      }
    },
    [
      busyAction,
      loadMountDetail,
      message,
      mountDetail,
      refreshLibrary,
      t,
    ]
  );

  const linkedProjectMount = useMemo(() => {
    const mountId = projectDetail?.summary.linked_mount_id;
    return mountId
      ? library?.plugins.find((plugin) => plugin.mount_id === mountId)
      : undefined;
  }, [library?.plugins, projectDetail?.summary.linked_mount_id]);

  const handleImportPrebuilt = useCallback(
    async (request: ImportPluginRequest) => {
      if (projectBusyAction) return;
      setProjectBusyAction('import');
      setProjectMutationFailure(null);
      try {
        const next = await ipcBridge.plugins.importPrebuilt.invoke(request);
        const nextLibrary = await ipcBridge.plugins.list.invoke();
        const installed = await ipcBridge.plugins.applyCandidate.invoke(
          applyPluginCandidateRequest(
            next,
            nextLibrary.library_revision,
            'initial_install',
            false,
            true,
            undefined
          )
        );
        setImportDialogVisible(false);
        setActiveTab('library');
        await refreshLibrary();
        setSelectedProjectId(next.summary.project_id);
        setProjectDetail(next);
        setSelectedMountId(installed.summary.mount_id);
        setMountDetail(installed);
        setSurface('detail');
        message.success(t('pluginWorkbench.messages.candidateApplied'));
      } catch (error) {
        console.error('[plugins] importing prebuilt Artifact failed', error);
        setProjectMutationFailure(pluginLoadFailure(error, 'resource'));
      } finally {
        setProjectBusyAction(null);
      }
    },
    [message, projectBusyAction, refreshLibrary, setActiveTab, t]
  );

  const handleBuildProject = useCallback(async () => {
    if (!projectDetail || projectBusyAction) return;
    setProjectBusyAction('build');
    setProjectMutationFailure(null);
    try {
      const next = await ipcBridge.plugins.buildProject.invoke(
        buildPluginProjectRequest(projectDetail)
      );
      setProjectDetail(next);
      await refreshLibrary();
      message.success(t('pluginWorkbench.messages.buildCompleted'));
    } catch (error) {
      console.error('[plugins] Project Build failed', error);
      setProjectMutationFailure(pluginLoadFailure(error, 'resource'));
    } finally {
      setProjectBusyAction(null);
    }
  }, [message, projectBusyAction, projectDetail, refreshLibrary, t]);

  const handleSourceEdit = useCallback(
    async (request: ApplyPluginSourceEditRequest) => {
      if (projectBusyAction) return;
      setProjectBusyAction('edit');
      setProjectMutationFailure(null);
      try {
        const next = await ipcBridge.plugins.applySourceEdit.invoke(request);
        setProjectDetail(next);
        setSourceEditDialogVisible(false);
        await refreshLibrary();
        message.success(t('pluginWorkbench.messages.sourceEdited'));
      } catch (error) {
        console.error('[plugins] source edit failed', error);
        setProjectMutationFailure(pluginLoadFailure(error, 'resource'));
      } finally {
        setProjectBusyAction(null);
      }
    },
    [message, projectBusyAction, refreshLibrary, t]
  );

  const handleDependencyUpdate = useCallback(
    async (request: UpdatePluginDependenciesRequest) => {
      if (projectBusyAction) return;
      setProjectBusyAction('dependencies');
      setProjectMutationFailure(null);
      try {
        const next = await ipcBridge.plugins.updateDependencies.invoke(request);
        setProjectDetail(next);
        setDependencyDialogVisible(false);
        await refreshLibrary();
        message.success(t('pluginWorkbench.messages.dependenciesUpdated'));
      } catch (error) {
        console.error('[plugins] dependency update failed', error);
        setProjectMutationFailure(pluginLoadFailure(error, 'resource'));
      } finally {
        setProjectBusyAction(null);
      }
    },
    [message, projectBusyAction, refreshLibrary, t]
  );

  const handleShareExport = useCallback(
    async (request: SharePluginRequest) => {
      if (projectBusyAction) return;
      setProjectBusyAction('share');
      setProjectMutationFailure(null);
      try {
        const operation = await ipcBridge.plugins.exportShare.invoke(request);
        setShareDialogVisible(false);
        await refreshLibrary();
        message.success(
          t('pluginWorkbench.messages.shareExported', {
            digest: operation.result_artifact_digests.share_bundle?.slice(0, 12) ?? '-',
          })
        );
      } catch (error) {
        console.error('[plugins] Share Bundle export failed', error);
        setProjectMutationFailure(pluginLoadFailure(error, 'resource'));
      } finally {
        setProjectBusyAction(null);
      }
    },
    [message, projectBusyAction, refreshLibrary, t]
  );

  const handleTestCandidate = useCallback(
    async (resolvedTestInputDigest: string) => {
      if (!projectDetail || projectBusyAction) return;
      setProjectBusyAction('test');
      setProjectMutationFailure(null);
      try {
        const mount = projectDetail.summary.linked_mount_id
          ? await ipcBridge.plugins.getMount.invoke({
              mount_id: projectDetail.summary.linked_mount_id,
            })
          : null;
        const next = await ipcBridge.plugins.testCandidate.invoke(
          testPluginCandidateRequest(
            projectDetail,
            mount?.config.config_revision ?? 0,
            mount?.credential_bindings_revision ?? 0,
            resolvedTestInputDigest
          )
        );
        setTestDialogVisible(false);
        setProjectDetail(next);
        await refreshLibrary();
        message.success(
          t(
            projectDetail.summary.apply_mode === 'auto_compatible_when_idle' &&
              projectDetail.ready &&
              !next.ready
              ? 'pluginWorkbench.messages.autoApplied'
              : 'pluginWorkbench.messages.candidateTested'
          )
        );
      } catch (error) {
        console.error('[plugins] Candidate Test failed', error);
        setProjectMutationFailure(pluginLoadFailure(error, 'resource'));
      } finally {
        setProjectBusyAction(null);
      }
    },
    [message, projectBusyAction, projectDetail, refreshLibrary, t]
  );

  const handleApplyCandidate = useCallback(
    async ({
      target,
      allowBreaking,
      acknowledgeTestWarning,
    }: {
      target: PluginApplyTargetSelection;
      allowBreaking: boolean;
      acknowledgeTestWarning: boolean;
    }) => {
      if (!projectDetail || !library || projectBusyAction) return;
      setProjectBusyAction('apply');
      setProjectMutationFailure(null);
      try {
        const next = await ipcBridge.plugins.applyCandidate.invoke(
          applyPluginCandidateRequest(
            projectDetail,
            library.library_revision,
            target,
            allowBreaking,
            acknowledgeTestWarning,
            linkedProjectMount
          )
        );
        setApplyDialogVisible(false);
        setMountDetail(next);
        setSelectedMountId(next.summary.mount_id);
        setActiveTab('library');
        setSurface('detail');
        await refreshLibrary();
        message.success(t('pluginWorkbench.messages.candidateApplied'));
      } catch (error) {
        console.error('[plugins] Candidate Apply failed', error);
        setProjectMutationFailure(pluginLoadFailure(error, 'resource'));
      } finally {
        setProjectBusyAction(null);
      }
    },
    [
      library,
      linkedProjectMount,
      message,
      projectBusyAction,
      projectDetail,
      refreshLibrary,
      setActiveTab,
      t,
    ]
  );

  const productItems = useMemo(
    () => (library ? pluginProductItems(library, runtimeDrafts).filter((item) => pluginProductMatches(item, searchQuery)) : []),
    [library, runtimeDrafts, searchQuery]
  );

  const openProductItem = useCallback((item: PluginProductItem) => {
    if (item.runtimeDraft && !item.runtime?.releases.active) {
      navigate(`/plugins/create/${item.runtimeDraft.id}`);
      return;
    }
    if (item.runtime) {
      navigate(`/plugins/run/${item.runtime.plugin_id}`);
      return;
    }
    if (item.mount) {
      setSurface('detail');
      setActiveTab('library');
      if (selectedMountId !== item.mount.mount_id) {
        setMountDetail(null);
      }
      setSelectedMountId(item.mount.mount_id);
      return;
    }
    if (item.project) {
      setSurface('creator');
      setActiveTab('workshop');
      setSelectedProjectId(item.project.project_id);
      setProjectDetail(null);
      setCreatorMessages([]);
      setAiDraft(null);
    }
  }, [navigate, selectedMountId, setActiveTab]);

  const runAiAuthoring = useCallback(async (requirement: string) => {
    const normalized = requirement.trim();
    if (!normalized) return;
    setSurface('creator');
    setActiveTab('workshop');
    setCreatorMessages((current) => [...current, { role: 'user', content: normalized }]);
    setProjectMutationFailure(null);
    if (!current_model) {
      setProjectMutationFailure({
        kind: 'unavailable',
        message: t('pluginWorkbench.product.modelRequired'),
      });
      return;
    }
    setAiBusyStep('writing');
    try {
      const currentContext = projectDetail && !aiDraft
        ? await ipcBridge.plugins.getAuthoringContext.invoke({
            project_id: projectDetail.summary.project_id,
          })
        : null;
      const packageId = aiDraft?.package_id ?? currentContext?.package_id ?? pluginPackageId();
      const packageVersion = aiDraft?.package_version ?? currentContext?.package_version ?? '0.1.0';
      const generated = await ipcBridge.plugins.generateDraft.invoke({
        provider_id: String(current_model.id),
        model: current_model.use_model,
        requirement: normalized,
        package_id: packageId,
        package_version: packageVersion,
        current_source: aiDraft?.source_content ?? currentContext?.source_content,
      });
      setAiDraft(generated);

      let nextProject = projectDetail;
      if (!nextProject) {
        if (!library) throw new Error('PLUGIN_LIBRARY_UNAVAILABLE');
        nextProject = await ipcBridge.plugins.createProject.invoke({
          expected_library_revision: library.library_revision,
          package_id: generated.package_id,
          package_version: generated.package_version,
          display_name: generated.display_name,
          description: generated.description,
          language: generated.language,
        });
        setSelectedProjectId(nextProject.summary.project_id);
      }

      setAiBusyStep('dependencies');
      for (const edit of pluginDraftSourceEdits(generated)) {
        if (!nextProject.source_snapshot_digest) {
          throw new Error('PLUGIN_SOURCE_SNAPSHOT_UNAVAILABLE');
        }
        nextProject = await ipcBridge.plugins.applySourceEdit.invoke({
          project_id: nextProject.summary.project_id,
          expected_source_snapshot_digest: nextProject.source_snapshot_digest,
          edit: { kind: 'replace', path: edit.path, content: edit.content },
        });
      }
      if (!nextProject.source_snapshot_digest || !nextProject.dependency_lock_digest) {
        throw new Error('PLUGIN_DEPENDENCY_STATE_UNAVAILABLE');
      }
      nextProject = await ipcBridge.plugins.updateDependencies.invoke({
        project_id: nextProject.summary.project_id,
        expected_project_revision: nextProject.summary.project_revision,
        expected_build_generation: nextProject.summary.build_generation,
        expected_source_snapshot_digest: nextProject.source_snapshot_digest,
        expected_dependency_lock_digest: nextProject.dependency_lock_digest,
        dependencies: generated.dependencies,
      });

      setAiBusyStep('building');
      nextProject = await ipcBridge.plugins.buildProject.invoke(
        buildPluginProjectRequest(nextProject)
      );
      setProjectDetail(nextProject);
      setCreatorMessages((current) => [
        ...current,
        { role: 'assistant', content: generated.assistant_message },
      ]);
      message.success(
        t(projectDetail ? 'pluginWorkbench.product.aiUpdated' : 'pluginWorkbench.product.aiGenerated')
      );
      await refreshLibrary();
    } catch (error) {
      console.error('[plugins] AI authoring failed', error);
      setProjectMutationFailure({
        kind: 'error',
        message: t('pluginWorkbench.product.aiTemporaryFailure'),
      });
      setCreatorMessages((current) => [
        ...current,
        { role: 'assistant', content: t('pluginWorkbench.product.aiFailed') },
      ]);
    } finally {
      setAiBusyStep(null);
    }
  }, [
    aiDraft,
    current_model,
    library,
    message,
    projectDetail,
    refreshLibrary,
    setActiveTab,
    t,
  ]);


  const toggleProductItem = useCallback(async (item: PluginProductItem, enabled: boolean) => {
    if ((!item.mount && !item.runtime) || homeBusyMountId) return;
    setHomeBusyMountId(item.key);
    try {
      if (item.runtime) {
        const detail = await ipcBridge.pluginRuntimes.getWorkshop.invoke({ plugin_id: item.runtime.plugin_id });
        const request = pluginRuntimeSetEnabledRequest(detail, enabled);
        if (!request) throw new Error(t('pluginWorkbench.states.errorBody'));
        await ipcBridge.pluginRuntimes.setEnabled.invoke(request);
      } else if (item.mount) {
        const detail = await ipcBridge.plugins.getMount.invoke({ mount_id: item.mount.mount_id });
        await ipcBridge.plugins.setEnabled.invoke(setPluginEnabledRequest(detail, enabled));
      }
      message.success(t(enabled ? 'pluginWorkbench.messages.enabled' : 'pluginWorkbench.messages.disabled'));
      await refreshLibrary();
      libraryChanged();
    } catch (error) {
      setMutationFailure(pluginLoadFailure(error, 'resource'));
    } finally {
      setHomeBusyMountId(null);
    }
  }, [homeBusyMountId, message, refreshLibrary, t]);

  const continueLinkedProject = useCallback(() => {
    const projectId = mountDetail?.summary.linked_project_id;
    if (!projectId) return;
    setSurface('creator');
    setActiveTab('workshop');
    setSelectedProjectId(projectId);
    setProjectDetail(null);
  }, [mountDetail?.summary.linked_project_id, setActiveTab]);

  const openAgentCapability = useCallback((capabilityId?: string, presetId?: string) => {
    const query = new URLSearchParams({ source: 'plugin' });
    if (capabilityId) query.set('capability', capabilityId);
    if (presetId) query.set('preset', presetId);
    void navigate(`/agent?${query.toString()}`);
  }, [navigate]);

  const backToPluginHome = useCallback(() => {
    setSurface('home');
    setActiveTab('library');
    if (searchParams.has('plugin')) { const next = new URLSearchParams(searchParams); next.delete('plugin'); next.delete('tab'); setSearchParams(next, {replace: true}); }
  }, [setActiveTab, searchParams, setSearchParams]);


  return (
    <HubPageShell
      title={t('pluginWorkbench.product.title')}
      hideHeading
      maxWidthClass='md:max-w-1440px'
      className={styles.pageShell}
    >
      {messageContext}
      <>
          {failure && library && (
          <Alert
            type={failure.kind === 'unavailable' ? 'warning' : 'error'}
            showIcon
            title={
              failure.kind === 'unavailable'
                ? t('pluginWorkbench.states.unavailableTitle')
                : t('pluginWorkbench.states.errorTitle')
            }
            content={failure.message}
            action={
              <Button size='small' onClick={() => void refreshVisible()}>
                {t('pluginWorkbench.actions.retry')}
              </Button>
            }
            className='mb-12px'
          />
          )}
          {surface === 'home' && mutationFailure && <Alert type='error' showIcon content={mutationFailure.message} />}
          {loading && !library ? (
            <PluginStatePanel loading body={t('pluginWorkbench.states.loadingLibrary')} />
          ) : library ? (
            surface === 'home' ? (
              <PluginProductHome
                items={productItems}
                loading={loading}
                search={searchQuery}
                busyMountId={homeBusyMountId}
                onSearch={setSearchQuery}
                pinnedIds={new Set(Object.entries(workspace.items).filter(([, item]) => item.pinned).map(([id]) => id))}
                onPin={(item) => {
                  setMutationFailure(null);
                  const id = item.runtime?.plugin_id ?? item.mount?.mount_id;
                  if (id) void updatePluginRuntimeWorkspace((current) => {
                    const value = current.items[id] ?? emptyItem();
                    current.items[id] = { ...value, pinned: !value.pinned };
                  }).catch((error) => setMutationFailure(pluginLoadFailure(error, 'resource')));
                }}
                onCreate={(requirement) => navigate(`/plugins/new${requirement ? `?requirement=${encodeURIComponent(requirement)}` : ''}`)}
                onImport={() => {
                  setProjectMutationFailure(null);
                  setImportDialogVisible(true);
                }}
                onOpen={openProductItem}
                onToggleEnabled={(item, enabled) => void toggleProductItem(item, enabled)}
              />
            ) : surface === 'creator' ? (
              <PluginCreatorSurface
                title={aiDraft?.display_name ?? projectDetail?.summary.display_name ?? ''}
                messages={creatorMessages}
                draft={aiDraft}
                project={projectDetail}
                busyStep={aiBusyStep}
                failure={projectMutationFailure ?? projectFailure}
                modelAvailable={Boolean(current_model)}
                projectBusy={projectBusyAction !== null || projectDetailLoading}
                onBack={backToPluginHome}
                onSend={(value) => void runAiAuthoring(value)}
                onOpenModels={() => void navigate('/models?section=models')}
                onBuild={() => void handleBuildProject()}
                onEditSource={() => {
                  setProjectMutationFailure(null);
                  setSourceEditDialogVisible(true);
                }}
                onEditDependencies={() => {
                  setProjectMutationFailure(null);
                  setDependencyDialogVisible(true);
                }}
                onExport={() => {
                  setProjectMutationFailure(null);
                  setShareDialogVisible(true);
                }}
                onTest={() => setTestDialogVisible(true)}
                onApply={() => setApplyDialogVisible(true)}
              />
            ) : mountDetail ? (
              <PluginProductDetail
                detail={mountDetail}
                agentUsage={agentUsage}
                failure={mutationFailure ?? mountFailure}
                busyAction={busyAction}
                onBack={backToPluginHome}
                onContinueCreating={continueLinkedProject}
                onOpenAgent={openAgentCapability}
                onConfigure={() => {
                  setMutationFailure(null);
                  setConfigureDialogVisible(true);
                }}
                onEnable={() => void runMountMutation(
                  'enable',
                  (detail) => ipcBridge.plugins.setEnabled.invoke(setPluginEnabledRequest(detail, true)),
                  'pluginWorkbench.messages.enabled'
                )}
                onDisable={() => void runMountMutation(
                  'disable',
                  (detail) => ipcBridge.plugins.setEnabled.invoke(setPluginEnabledRequest(detail, false)),
                  'pluginWorkbench.messages.disabled'
                )}
                onRetry={() => void runMountMutation(
                  'retry',
                  (detail) => ipcBridge.plugins.retryMount.invoke(retryPluginRequest(detail)),
                  'pluginWorkbench.messages.retried'
                )}
                onRestore={() => void runMountMutation(
                  'restore',
                  (detail) => ipcBridge.plugins.restorePrevious.invoke(restorePluginRequest(detail)),
                  'pluginWorkbench.messages.restored'
                )}
                onUninstall={handleUninstall}
                onDeleteData={handleDeleteData}
              />
            ) : mountDetailLoading ? (
              <PluginStatePanel loading body={t('pluginWorkbench.states.loadingMount')} />
            ) : (
              <PluginStatePanel failure={mountFailure ?? {
                kind: 'error',
                message: t('pluginWorkbench.states.errorBody'),
              }} onRetry={() => selectedMountId && void loadMountDetail(selectedMountId)} />
            )
          ) : failure ? (
            <PluginStatePanel
              failure={failure}
              onRetry={() => void refreshVisible()}
            />
          ) : null}
      </>
      <PluginConfigurationDialog
        visible={configureDialogVisible}
        detail={mountDetail}
        loading={busyAction === 'configure'}
        failure={configureDialogVisible ? mutationFailure : null}
        onCancel={() => {
          setConfigureDialogVisible(false);
          setMutationFailure(null);
        }}
        onSubmit={handleConfigure}
      />
      <PluginSmartImportDialog
        visible={importDialogVisible}
        libraryRevision={library?.library_revision ?? 0}
        loading={projectBusyAction === 'import'}
        failure={importDialogVisible ? projectMutationFailure : null}
        onCancel={() => setImportDialogVisible(false)}
        onSubmit={handleImportPrebuilt}
      />
      <PluginCandidateTestModal
        visible={testDialogVisible}
        detail={projectDetail}
        loading={projectBusyAction === 'test'}
        failure={testDialogVisible ? projectMutationFailure : null}
        onCancel={() => setTestDialogVisible(false)}
        onSubmit={handleTestCandidate}
      />
      <PluginCandidateApplyModal
        visible={applyDialogVisible}
        detail={projectDetail}
        linkedMount={linkedProjectMount}
        loading={projectBusyAction === 'apply'}
        failure={applyDialogVisible ? projectMutationFailure : null}
        onCancel={() => setApplyDialogVisible(false)}
        onSubmit={handleApplyCandidate}
      />
      <PluginSourceEditDialog
        visible={sourceEditDialogVisible}
        detail={projectDetail}
        loading={projectBusyAction === 'edit'}
        failure={sourceEditDialogVisible ? projectMutationFailure : null}
        onCancel={() => {
          setSourceEditDialogVisible(false);
          setProjectMutationFailure(null);
        }}
        onSubmit={handleSourceEdit}
      />
      <PluginDependencyDialog
        visible={dependencyDialogVisible}
        detail={projectDetail}
        loading={projectBusyAction === 'dependencies'}
        failure={dependencyDialogVisible ? projectMutationFailure : null}
        onCancel={() => {
          setDependencyDialogVisible(false);
          setProjectMutationFailure(null);
        }}
        onSubmit={handleDependencyUpdate}
      />
      <PluginShareExportDialog
        visible={shareDialogVisible}
        detail={projectDetail}
        linkedMount={linkedProjectMount}
        loading={projectBusyAction === 'share'}
        failure={shareDialogVisible ? projectMutationFailure : null}
        onCancel={() => {
          setShareDialogVisible(false);
          setProjectMutationFailure(null);
        }}
        onSubmit={handleShareExport}
      />
    </HubPageShell>
  );
};

export default PluginWorkbenchPage;
