/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type {
  ConfigurePluginRequest,
  ApplyPluginSourceEditRequest,
  PluginDetail,
  PluginLibraryResponse,
  PluginMountId,
  PluginProjectDetail,
  PluginProjectId,
  CreatePluginProjectRequest,
  ImportPluginRequest,
  SharePluginRequest,
  UpdatePluginDependenciesRequest,
} from '@/common/types/pluginPlatform';
import { Alert, Button, Input, Modal } from '@arco-design/web-react';
import { AddOne, Code, Refresh, Search, Plug, Upload } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import HubPageShell from '@/renderer/components/layout/HubPageShell';
import SegmentedTabs from '@/renderer/components/base/SegmentedTabs';
import { isDesktopShell } from '@/renderer/utils/platform';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import PluginLibraryView, {
  type PluginMountBusyAction,
} from './PluginLibraryView';
import PluginConfigurationDialog from './PluginConfigurationDialog';
import PluginWorkshopView, {
  type PluginProjectBusyAction,
} from './PluginWorkshopView';
import {
  PluginCandidateApplyModal,
  PluginCandidateTestModal,
  PluginPrebuiltImportModal,
  PluginProjectCreateModal,
} from './PluginWorkbenchDialogs';
import PluginSourceEditDialog from './PluginSourceEditDialog';
import PluginDependencyDialog from './PluginDependencyDialog';
import PluginShareExportDialog from './PluginShareExportDialog';
import {
  applyPluginCandidateRequest,
  buildPluginProjectRequest,
  deletePluginDataRequest,
  deletePluginProjectRequest,
  pluginLoadFailure,
  restorePluginRequest,
  retryPluginRequest,
  setPluginAutoApplyRequest,
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

const PluginWorkbenchPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const [message, messageContext] = useArcoMessage({ maxCount: 8 });
  const [searchParams, setSearchParams] = useSearchParams();
  const activeTab: WorkbenchTab =
    searchParams.get('tab') === 'workshop' ? 'workshop' : 'library';
  const [library, setLibrary] = useState<PluginLibraryResponse | null>(null);
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
  const [createDialogVisible, setCreateDialogVisible] = useState(false);
  const [importDialogVisible, setImportDialogVisible] = useState(false);
  const [testDialogVisible, setTestDialogVisible] = useState(false);
  const [applyDialogVisible, setApplyDialogVisible] = useState(false);
  const [sourceEditDialogVisible, setSourceEditDialogVisible] = useState(false);
  const [dependencyDialogVisible, setDependencyDialogVisible] = useState(false);
  const [shareDialogVisible, setShareDialogVisible] = useState(false);
  const [configureDialogVisible, setConfigureDialogVisible] = useState(false);
  const mountLoadSequence = useRef(0);
  const projectLoadSequence = useRef(0);
  const desktop = isDesktopShell();

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
    if (!desktop) {
      setLoading(false);
      setFailure({
        kind: 'unavailable',
        message: t('pluginWorkbench.states.desktopOnlyBody'),
      });
      return;
    }

    setLoading(true);
    try {
      const next = await ipcBridge.plugins.list.invoke();
      setLibrary(next);
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
  }, [desktop, t]);

  useEffect(() => {
    void refreshLibrary();
  }, [refreshLibrary]);

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

  const filteredPlugins = useMemo(() => {
    const plugins = library?.plugins ?? [];
    const query = searchQuery.trim().toLowerCase();
    if (!query) return plugins;
    return plugins.filter((plugin) =>
      `${plugin.display_name} ${plugin.description ?? ''} ${plugin.current?.package_id ?? ''}`
        .toLowerCase()
        .includes(query)
    );
  }, [library?.plugins, searchQuery]);

  const filteredProjects = useMemo(() => {
    const projects = library?.projects ?? [];
    const query = searchQuery.trim().toLowerCase();
    if (!query) return projects;
    return projects.filter((project) =>
      `${project.display_name} ${project.description ?? ''} ${project.project_id}`
        .toLowerCase()
        .includes(query)
    );
  }, [library?.projects, searchQuery]);

  const hasSearchQuery = searchQuery.trim().length > 0;

  useEffect(() => {
    if (activeTab === 'library') {
      if (selectedMountId && filteredPlugins.some((plugin) => plugin.mount_id === selectedMountId)) {
        return;
      }
      const nextMountId = filteredPlugins[0]?.mount_id ?? null;
      if (nextMountId !== selectedMountId) {
        setSelectedMountId(nextMountId);
        setMountDetail(null);
      }
      return;
    }

    if (selectedProjectId && filteredProjects.some((project) => project.project_id === selectedProjectId)) {
      return;
    }
    const nextProjectId = filteredProjects[0]?.project_id ?? null;
    if (nextProjectId !== selectedProjectId) {
      setSelectedProjectId(nextProjectId);
      setProjectDetail(null);
    }
  }, [
    activeTab,
    filteredPlugins,
    filteredProjects,
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

  const handleCreateProject = useCallback(
    async (request: CreatePluginProjectRequest) => {
      if (projectBusyAction) return;
      setProjectBusyAction('create');
      setProjectMutationFailure(null);
      try {
        const next = await ipcBridge.plugins.createProject.invoke(request);
        setCreateDialogVisible(false);
        setActiveTab('workshop');
        await refreshLibrary();
        setSelectedProjectId(next.summary.project_id);
        setProjectDetail(next);
        message.success(t('pluginWorkbench.messages.projectCreated'));
      } catch (error) {
        console.error('[plugins] creating Project failed', error);
        setProjectMutationFailure(pluginLoadFailure(error, 'resource'));
      } finally {
        setProjectBusyAction(null);
      }
    },
    [message, projectBusyAction, refreshLibrary, setActiveTab, t]
  );

  const handleImportPrebuilt = useCallback(
    async (request: ImportPluginRequest) => {
      if (projectBusyAction) return;
      setProjectBusyAction('import');
      setProjectMutationFailure(null);
      try {
        const next = await ipcBridge.plugins.importPrebuilt.invoke(request);
        setImportDialogVisible(false);
        setActiveTab('workshop');
        await refreshLibrary();
        setSelectedProjectId(next.summary.project_id);
        setProjectDetail(next);
        message.success(t('pluginWorkbench.messages.artifactImported'));
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

  const handleSetAutoApply = useCallback(
    (enabled: boolean) => {
      if (!projectDetail || projectBusyAction) return;
      const execute = async () => {
        const mount = linkedProjectMount;
        if (enabled && (!mount?.current || !projectDetail.summary.linked_mount_id)) {
          setProjectMutationFailure({
            kind: 'resource',
            message: t('pluginWorkbench.autoApply.linkedMountUnavailable'),
          });
          return;
        }
        const request = setPluginAutoApplyRequest(projectDetail, enabled, mount);
        setProjectBusyAction('auto_apply');
        setProjectMutationFailure(null);
        try {
          const next = await ipcBridge.plugins.setAutoApply.invoke(request);
          setProjectDetail(next);
          await refreshLibrary();
          message.success(
            t(
              enabled
                ? projectDetail.ready && !next.ready
                  ? 'pluginWorkbench.messages.autoApplied'
                  : next.ready
                    ? 'pluginWorkbench.messages.autoApplyWaiting'
                    : 'pluginWorkbench.messages.autoApplyEnabled'
                : 'pluginWorkbench.messages.autoApplyDisabled'
            )
          );
        } catch (error) {
          console.error('[plugins] updating auto Apply authorization failed', error);
          setProjectMutationFailure(pluginLoadFailure(error, 'resource'));
        } finally {
          setProjectBusyAction(null);
        }
      };
      if (enabled && projectDetail.summary.apply_mode !== 'auto_compatible_when_idle') {
        Modal.confirm({
          title: t('pluginWorkbench.autoApply.confirmTitle'),
          content: t('pluginWorkbench.autoApply.confirmBody'),
          okText: t('pluginWorkbench.actions.enableAutoApply'),
          cancelText: t('pluginWorkbench.actions.cancel'),
          onOk: execute,
        });
      } else {
        void execute();
      }
    },
    [
      linkedProjectMount,
      message,
      projectBusyAction,
      projectDetail,
      refreshLibrary,
      t,
    ]
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

  const handleCancelProjectOperation = useCallback(async () => {
    const operation = projectDetail?.active_operation;
    if (!operation || projectBusyAction) return;
    setProjectBusyAction('cancel_operation');
    setProjectMutationFailure(null);
    try {
      await ipcBridge.plugins.cancelOperation.invoke({
        operation_id: operation.operation_id,
        expected_operation_revision: operation.operation_revision,
      });
      if (selectedProjectId) {
        await loadProjectDetail(selectedProjectId);
      }
      await refreshLibrary();
      message.success(t('pluginWorkbench.messages.operationCanceled'));
    } catch (error) {
      console.error('[plugins] canceling Project Operation failed', error);
      setProjectMutationFailure(pluginLoadFailure(error, 'resource'));
    } finally {
      setProjectBusyAction(null);
    }
  }, [
    loadProjectDetail,
    message,
    projectBusyAction,
    projectDetail?.active_operation,
    refreshLibrary,
    selectedProjectId,
    t,
  ]);

  const handleDeleteProject = useCallback(() => {
    if (!projectDetail || projectBusyAction) return;
    Modal.confirm({
      title: t('pluginWorkbench.confirm.deleteProjectTitle'),
      content: t('pluginWorkbench.confirm.deleteProjectBody'),
      okText: t('pluginWorkbench.actions.deleteProject'),
      cancelText: t('pluginWorkbench.actions.cancel'),
      okButtonProps: { status: 'danger' },
      onOk: async () => {
        setProjectBusyAction('delete');
        setProjectMutationFailure(null);
        try {
          await ipcBridge.plugins.deleteProject.invoke(
            deletePluginProjectRequest(projectDetail)
          );
          setProjectDetail(null);
          setSelectedProjectId(null);
          await refreshLibrary();
          message.success(t('pluginWorkbench.messages.projectDeleted'));
        } catch (error) {
          console.error('[plugins] deleting Project failed', error);
          setProjectMutationFailure(pluginLoadFailure(error, 'resource'));
        } finally {
          setProjectBusyAction(null);
        }
      },
    });
  }, [message, projectBusyAction, projectDetail, refreshLibrary, t]);

  const tabItems = [
    {
      key: 'library',
      label: t('pluginWorkbench.tabs.library'),
      icon: <Plug theme='outline' size='15' />,
    },
    {
      key: 'workshop',
      label: t('pluginWorkbench.tabs.workshop'),
      icon: <Code theme='outline' size='15' />,
      dot: (library?.projects.length ?? 0) > 0 && (library?.plugins.length ?? 0) === 0,
    },
  ];

  const desktopOnlyState = !desktop ? (
    <PluginStatePanel
      title={t('pluginWorkbench.states.desktopOnlyTitle')}
      body={t('pluginWorkbench.states.desktopOnlyBody')}
    />
  ) : null;

  return (
    <HubPageShell
      title={t('pluginWorkbench.title')}
      subtitle={t('pluginWorkbench.subtitle')}
      maxWidthClass='md:max-w-1440px'
      className={styles.pageShell}
      toolbar={
        <div className={styles.toolbar}>
          <SegmentedTabs
            items={tabItems}
            activeKey={activeTab}
            onChange={(key) => {
              if (key === 'library' || key === 'workshop') setActiveTab(key);
            }}
            size='sm'
          />
          <div className={styles.toolbarMeta}>
            <Button
              size='small'
              icon={<AddOne size={14} fill='currentColor' />}
              disabled={loading || projectBusyAction !== null}
              onClick={() => {
                setProjectMutationFailure(null);
                setCreateDialogVisible(true);
              }}
            >
              {t('pluginWorkbench.actions.createProject')}
            </Button>
            <Button
              size='small'
              icon={<Upload size={14} fill='currentColor' />}
              disabled={loading || projectBusyAction !== null}
              onClick={() => {
                setProjectMutationFailure(null);
                setImportDialogVisible(true);
              }}
            >
              {t('pluginWorkbench.actions.importArtifact')}
            </Button>
            <Input
              value={searchQuery}
              onChange={setSearchQuery}
              placeholder={t('pluginWorkbench.actions.search')}
              className='!w-220px'
              size='small'
              prefix={<Search size={13} fill='currentColor' />}
              allowClear
            />
            <Button
              type='text'
              size='small'
              icon={<Refresh size={15} fill='currentColor' />}
              loading={loading}
              onClick={() => void refreshVisible()}
              title={t('pluginWorkbench.actions.refresh')}
              aria-label={t('pluginWorkbench.actions.refresh')}
            />
          </div>
        </div>
      }
    >
      {messageContext}
      {!desktop ? (
        desktopOnlyState
      ) : (
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
          {loading && !library ? (
            <PluginStatePanel loading body={t('pluginWorkbench.states.loadingLibrary')} />
          ) : library ? (
            activeTab === 'library' ? (
              <PluginLibraryView
                plugins={filteredPlugins}
                selectedMountId={selectedMountId}
                detail={mountDetail ?? undefined}
                detailLoading={mountDetailLoading}
                detailFailure={mountFailure}
                mutationFailure={mutationFailure}
                busyAction={busyAction}
                locale={i18n.resolvedLanguage ?? i18n.language}
                onSelect={(mountId) => {
                  setSelectedMountId(mountId);
                  setMountDetail(null);
                }}
                onRetryDetail={() => {
                  if (selectedMountId) void loadMountDetail(selectedMountId);
                }}
                onConfigure={() => {
                  setMutationFailure(null);
                  setConfigureDialogVisible(true);
                }}
                onEnable={() =>
                  void runMountMutation(
                    'enable',
                    (detail) =>
                      ipcBridge.plugins.setEnabled.invoke(setPluginEnabledRequest(detail, true)),
                    'pluginWorkbench.messages.enabled'
                  )
                }
                onDisable={() =>
                  void runMountMutation(
                    'disable',
                    (detail) =>
                      ipcBridge.plugins.setEnabled.invoke(setPluginEnabledRequest(detail, false)),
                    'pluginWorkbench.messages.disabled'
                  )
                }
                onRetryMount={() =>
                  void runMountMutation(
                    'retry',
                    (detail) => ipcBridge.plugins.retryMount.invoke(retryPluginRequest(detail)),
                    'pluginWorkbench.messages.retried'
                  )
                }
                onRestore={() =>
                  void runMountMutation(
                    'restore',
                    (detail) =>
                      ipcBridge.plugins.restorePrevious.invoke(restorePluginRequest(detail)),
                    'pluginWorkbench.messages.restored'
                  )
                }
                onUninstall={handleUninstall}
                onDeleteData={handleDeleteData}
              />
            ) : (
              <PluginWorkshopView
                projects={filteredProjects}
                hasQuery={hasSearchQuery}
                selectedProjectId={selectedProjectId}
                detail={projectDetail ?? undefined}
                detailLoading={projectDetailLoading}
                detailFailure={projectFailure}
                mutationFailure={projectMutationFailure}
                busyAction={projectBusyAction}
                locale={i18n.resolvedLanguage ?? i18n.language}
                onSelect={(projectId) => {
                  setSelectedProjectId(projectId);
                  setProjectDetail(null);
                }}
                onRetryDetail={() => {
                  if (selectedProjectId) void loadProjectDetail(selectedProjectId);
                }}
                onOpenMount={(mountId) => {
                  setActiveTab('library');
                  setSelectedMountId(mountId);
                  setMountDetail(null);
                }}
                onBuild={() => void handleBuildProject()}
                onEditSource={() => {
                  setProjectMutationFailure(null);
                  setSourceEditDialogVisible(true);
                }}
                onEditDependencies={() => {
                  setProjectMutationFailure(null);
                  setDependencyDialogVisible(true);
                }}
                onSetAutoApply={handleSetAutoApply}
                onExportShare={() => {
                  setProjectMutationFailure(null);
                  setShareDialogVisible(true);
                }}
                onTest={() => setTestDialogVisible(true)}
                onApply={() => setApplyDialogVisible(true)}
                onDelete={handleDeleteProject}
                onCancelOperation={() => void handleCancelProjectOperation()}
              />
            )
          ) : failure ? (
            <PluginStatePanel
              failure={failure}
              onRetry={() => void refreshVisible()}
            />
          ) : null}
        </>
      )}
      <PluginProjectCreateModal
        visible={createDialogVisible}
        libraryRevision={library?.library_revision ?? 0}
        loading={projectBusyAction === 'create'}
        failure={createDialogVisible ? projectMutationFailure : null}
        onCancel={() => setCreateDialogVisible(false)}
        onSubmit={handleCreateProject}
      />
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
      <PluginPrebuiltImportModal
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
