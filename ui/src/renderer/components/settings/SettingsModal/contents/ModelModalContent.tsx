/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IProvider, ModelTask } from '@/common/config/storage';
import type {
  ProviderModelCapabilityResponse,
  ProviderModelResponse,
} from '@/common/types/provider/providerModel';
import type { ProviderId } from '@/common/types/ids';
import { Button, Collapse, Divider, Input, Message, Modal, Popconfirm, Popover, Switch, Tag, Tooltip } from '@arco-design/web-react';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { Copy, DeleteFour, Info, Minus, Plus, Write, Heartbeat, Drag } from '@icon-park/react';
import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import {
  featureRoute,
  groupUsagesByFeature,
  parseProviderInUseDetails,
  type ProviderUsageFeature,
} from './providerInUse';
import AddModelModal from '@/renderer/pages/settings/components/AddModelModal';
import AddPlatformModal from '@/renderer/pages/settings/components/AddPlatformModal';
import ModelAdvancedEditor from '@/renderer/pages/settings/components/ModelAdvancedEditor';
import ProviderConnectionsSection from '@/renderer/pages/settings/components/ProviderConnectionsSection';
import EditModeModal from '@/renderer/pages/settings/components/EditModeModal';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { useContainerWidth } from '@/renderer/hooks/ui/useContainerWidth';
import ModelHubPageHeader from '@/renderer/pages/modelHub/ModelHubPageHeader';
import { consumePendingDeepLink } from '@/renderer/hooks/system/useDeepLink';
import { isManagedModelProvider } from '@/common/types/provider/managedModelService';
import { reorderById, reorderStrings } from './modelProviderOrdering';
import { healthFailureHeadline } from './healthFailureHeadline';
import {
  capabilityInputFromResponse,
  type ProviderModelCapabilityInput,
} from '@/renderer/pages/settings/components/providerModelAdvanced';
import '../model-provider.css';

/**
 * Health probe entry for one model row. A single-task model stays one click;
 * multi-task models require an explicit task so the probe cannot silently
 * exercise only the first advertised modality.
 */
const ModelHealthCheckAction: React.FC<{
  tasks: readonly ModelTask[];
  loading: boolean;
  onCheck: (task: ModelTask) => Promise<void>;
}> = ({ tasks, loading, onCheck }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const label = t('settings.healthCheck');
  const buttonClass =
    '!w-28px !h-28px !min-w-28px !bg-[var(--color-bg-1)] text-t-secondary hover:text-t-primary hover:!bg-[var(--fill-0)]';

  if (tasks.length <= 1) {
    const task = tasks[0];
    return (
      <Tooltip content={label}>
        <Button
          size='mini'
          className={buttonClass}
          icon={<Heartbeat theme='outline' size='16' />}
          loading={loading}
          disabled={!task}
          aria-label={label}
          onClick={() => task && void onCheck(task)}
        />
      </Tooltip>
    );
  }

  return (
    <Popover
      trigger='click'
      position='bl'
      popupVisible={open}
      onVisibleChange={(visible) => {
        if (!loading) setOpen(visible);
      }}
      content={
        <div
          className='flex flex-col gap-4px min-w-180px'
          role='menu'
          aria-label={label}
          data-health-task-menu
          onClick={(event) => event.stopPropagation()}
        >
          <div className='px-8px pb-2px text-12px text-t-secondary'>
            {label} · {t('settings.modelModality', { defaultValue: '模态能力' })}
          </div>
          {tasks.map((task) => (
            <Button
              key={task}
              type='text'
              size='small'
              long
              role='menuitem'
              className='!justify-start'
              disabled={loading}
              data-health-task={task}
              onClick={() => {
                setOpen(false);
                void onCheck(task);
              }}
            >
              {t(`settings.modelTask.${task}`, { defaultValue: task })}
            </Button>
          ))}
        </div>
      }
    >
      <Tooltip content={label}>
        <Button
          size='mini'
          className={buttonClass}
          icon={<Heartbeat theme='outline' size='16' />}
          loading={loading}
          aria-label={label}
          aria-haspopup='menu'
          aria-expanded={open}
        />
      </Tooltip>
    </Popover>
  );
};

/** One badge reflects one persisted task capability, never the row aggregate. */
const CapabilityHealthTag: React.FC<{ capability: ProviderModelCapabilityResponse }> = ({ capability }) => {
  const { t } = useTranslation();
  const capabilityHealth = capability.health;
  const status = capabilityHealth?.status ?? 'unknown';
  const color = status === 'healthy' ? 'green' : status === 'unhealthy' ? 'red' : capability.task === 'chat' ? 'gray' : 'purple';

  return (
    <Tooltip
      content={
        <div data-capability-health-tooltip={capability.task}>
          <div>
            {t(`settings.modelTask.${capability.task}`, { defaultValue: capability.task })}:{' '}
            {status === 'healthy'
              ? t('common.success')
              : status === 'unhealthy'
                ? t('common.failed')
                : t('settings.healthNotChecked', { defaultValue: 'Not checked' })}
          </div>
          {capabilityHealth?.latency !== undefined && (
            <div className='text-12px mt-4px'>
              {t('settings.latency')}: {capabilityHealth.latency}ms
            </div>
          )}
          {capabilityHealth?.error && <div className='text-12px mt-4px'>{capabilityHealth.error}</div>}
          {capability.health_checked_at !== undefined && (
            <div className='text-12px mt-4px'>
              {t('mcp.lastCheck')}: {new Date(capability.health_checked_at).toLocaleString()}
            </div>
          )}
        </div>
      }
    >
      <Tag
        size='small'
        color={color}
        bordered
        className='shrink-0 select-none'
        data-capability-health-task={capability.task}
        data-capability-health-status={status}
      >
        <span
          className={`inline-block w-6px h-6px mr-4px rounded-full ${
            status === 'healthy' ? 'bg-green-500' : status === 'unhealthy' ? 'bg-red-500' : 'bg-current opacity-35'
          }`}
          aria-hidden='true'
        />
        {t(`settings.modelTask.${capability.task}`, { defaultValue: capability.task })}
      </Tag>
    </Tooltip>
  );
};

/** ProviderResponse.models is the only authoritative model collection. */
const modelRowsFor = (platform: IProvider): ProviderModelResponse[] => platform.models;

/** Row-level partial update body minus the composite key. */
type ModelRowPatch = Partial<Pick<ProviderModelResponse, 'enabled' | 'sort_order'>> & {
  description?: string | null;
};

/** Apply a tri-state row patch to the cached row for optimistic rendering. */
const applyRowPatch = (row: ProviderModelResponse, patch: ModelRowPatch): ProviderModelResponse => ({
  ...row,
  ...(patch.enabled !== undefined ? { enabled: patch.enabled } : {}),
  ...(patch.sort_order !== undefined ? { sort_order: patch.sort_order } : {}),
  ...('description' in patch ? { description: patch.description ?? undefined } : {}),
});

const providerModelInputFor = (row: ProviderModelResponse) => ({
  model: row.model,
  enabled: row.enabled,
  sort_order: row.sort_order,
  ...(row.description ? { description: row.description } : {}),
  capabilities: row.capabilities.map(capabilityInputFromResponse),
});

/**
 * 每模型描述编辑浮层 / Per-model description editor popover.
 * 描述用于智能协作自动选择模型；空态显示占位提示。
 * The description drives automatic model selection for collaboration.
 */
const ModelDescriptionEditor: React.FC<{
  description: string;
  onSave: (text: string) => void;
}> = ({ description, onSave }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState(description);

  const placeholder = t('settings.modelDescriptionPlaceholder', {
    defaultValue: '描述该模型擅长什么，用于协作任务自动选择',
  });

  // 每次打开时同步最新描述，避免外部更新后草稿陈旧
  // Re-sync draft when opening so external updates aren't masked by stale draft.
  const handleVisibleChange = (visible: boolean) => {
    if (visible) setDraft(description);
    setOpen(visible);
  };

  const handleSave = () => {
    const next = draft.trim();
    if (next !== (description ?? '').trim()) {
      onSave(next);
    }
    setOpen(false);
  };

  return (
    <Popover
      trigger='click'
      position='bl'
      popupVisible={open}
      onVisibleChange={handleVisibleChange}
      content={
        <div className='flex flex-col gap-8px w-280px' onClick={(e) => e.stopPropagation()}>
          <div className='text-12px text-t-secondary'>
            {t('settings.modelDescriptionTitle', { defaultValue: '模型描述（用于智能协作）' })}
          </div>
          <Input.TextArea
            autoFocus
            value={draft}
            onChange={setDraft}
            placeholder={placeholder}
            autoSize={{ minRows: 3, maxRows: 6 }}
          />
          <div className='flex items-center justify-end gap-8px'>
            <Button size='mini' onClick={() => setOpen(false)}>
              {t('common.cancel', { defaultValue: '取消' })}
            </Button>
            <Button size='mini' type='primary' onClick={handleSave}>
              {t('common.save', { defaultValue: '保存' })}
            </Button>
          </div>
        </div>
      }
    >
      <Tooltip content={t('settings.editModelDescription', { defaultValue: '编辑模型描述' })}>
        <Button
          size='mini'
          className={`model-provider-action-btn !w-24px !h-24px !min-w-24px shrink-0 ${description ? 'text-primary-6 hover:text-primary-5' : 'text-t-secondary hover:text-t-primary'}`}
          icon={<Write theme='outline' size='14' />}
          onClick={(e) => e.stopPropagation()}
        />
      </Tooltip>
    </Popover>
  );
};

const providerSortableId = (providerId: ProviderId) => `provider:${providerId}`;
const modelSortableId = (providerId: ProviderId, model: string) => `model:${providerId}:${model}`;

type SortableDragData =
  | { type: 'provider'; providerId: ProviderId }
  | { type: 'model'; providerId: ProviderId; model: string };

type SortableRenderProps = {
  attributes: ReturnType<typeof useSortable>['attributes'];
  listeners: ReturnType<typeof useSortable>['listeners'];
  setActivatorNodeRef: ReturnType<typeof useSortable>['setActivatorNodeRef'];
  isDragging: boolean;
};

const SortableProviderCard: React.FC<{
  provider: IProvider;
  children: (props: SortableRenderProps) => React.ReactNode;
}> = ({ provider, children }) => {
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, transition, isDragging } = useSortable({
    id: providerSortableId(provider.id),
    data: { type: 'provider', providerId: provider.id } satisfies SortableDragData,
  });

  return (
    <div
      ref={setNodeRef}
      className={isDragging ? 'model-provider-sortable-card is-dragging' : 'model-provider-sortable-card'}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
      }}
    >
      {children({ attributes, listeners, setActivatorNodeRef, isDragging })}
    </div>
  );
};

const SortableModelRow: React.FC<{
  providerId: ProviderId;
  model: string;
  children: (props: SortableRenderProps) => React.ReactNode;
}> = ({ providerId, model, children }) => {
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, transition, isDragging } = useSortable({
    id: modelSortableId(providerId, model),
    data: { type: 'model', providerId, model } satisfies SortableDragData,
  });

  return (
    <div
      ref={setNodeRef}
      className={isDragging ? 'model-provider-sortable-row is-dragging' : 'model-provider-sortable-row'}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
      }}
    >
      {children({ attributes, listeners, setActivatorNodeRef, isDragging })}
    </div>
  );
};

const PriorityDragHandle: React.FC<SortableRenderProps & { label: string }> = ({
  attributes,
  listeners,
  setActivatorNodeRef,
  isDragging,
  label,
}) => (
  <Tooltip content={label}>
    <span
      ref={setActivatorNodeRef}
      {...attributes}
      {...listeners}
      aria-label={label}
      className={`model-provider-drag-handle inline-flex shrink-0 ${isDragging ? 'is-dragging' : ''}`}
      style={{ touchAction: 'none' }}
      onClick={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <Button
        tabIndex={-1}
        size='mini'
        className='model-provider-action-btn !w-24px !h-24px !min-w-24px text-t-secondary hover:text-t-primary cursor-grab'
        icon={<Drag theme='outline' size='14' />}
      />
    </span>
  </Tooltip>
);

const ModelModalContent: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  // 以「内容面板实际宽度」而非视口宽度做分档：模型管理面板被一次 rail + 二级
  // ContentSider 占去宽度，视口断点(md:/lg:)会误判为宽屏。窄面板下用紧凑布局，
  // 避免 provider 头 hover 展开区(320px)挤占供应商名称。
  const { ref: paneRef, width: paneWidth } = useContainerWidth<HTMLDivElement>();
  const isWide = paneWidth >= 520;
  const [collapseKey, setCollapseKey] = useState<Record<string, boolean>>({});
  const [healthCheckLoading, setHealthCheckLoading] = useState<Record<string, boolean>>({});
  const { data, mutate } = useProvidersQuery();
  // Managed providers have dedicated pages. Keeping them out of generic CRUD
  // prevents exposing or accidentally overwriting their internal endpoint and
  // per-boot credential.
  const editableProviders = useMemo(() => (data ?? []).filter((provider) => !isManagedModelProvider(provider)), [data]);
  const [message, messageContext] = useArcoMessage();
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates })
  );
  const providerSortableItems = useMemo(
    () => editableProviders.map((platform) => providerSortableId(platform.id)),
    [editableProviders]
  );

  /**
   * 行级模型更新：统一走 providerModel.save 全量模型聚合写。
   * Every model write is a full canonical upsert. Capability omission is a
   * deletion, so even metadata edits resend the complete visible capability set.
   */
  const updateModelRow = (
    platform: IProvider,
    row: ProviderModelResponse,
    patch: ModelRowPatch,
    rethrow = false
  ): Promise<void> => {
    const nextRow = applyRowPatch(row, patch);
    if (data) {
      const nextArray = data.map((item) =>
        item.id === platform.id
          ? {
              ...item,
              models: item.models.map((candidate) => (candidate.model === row.model ? nextRow : candidate)),
            }
          : item
      );
      void mutate(nextArray, false);
    }
    return ipcBridge.providerModel.save
      .invoke({ provider_id: platform.id, model: providerModelInputFor(nextRow) })
      .then(() => {
        void mutate();
      })
      .catch((error) => {
        void mutate();
        console.error('Failed to update model row:', error);
        message.error(t('settings.saveModelConfigFailed'));
        if (rethrow) throw error;
      });
  };

  const updateModelCapabilities = async (
    platform: IProvider,
    row: ProviderModelResponse,
    capabilities: ProviderModelCapabilityInput[]
  ): Promise<void> => {
    try {
      await ipcBridge.providerModel.save.invoke({
        provider_id: platform.id,
        model: { ...providerModelInputFor(row), capabilities },
      });
      await mutate();
    } catch (error) {
      console.error('Failed to save model capabilities:', error);
      message.error(t('settings.saveModelConfigFailed'));
      throw error;
    }
  };

  /** Delete one canonical model aggregate. */
  const removeModel = (platform: IProvider, model: string) => {
    if (data) {
      const nextArray = data.map((item) =>
        item.id === platform.id
          ? {
              ...item,
              models: item.models.filter((row) => row.model !== model),
            }
          : item
      );
      void mutate(nextArray, false);
    }
    ipcBridge.providerModel.remove
      .invoke({ provider_id: platform.id, model })
      .then(() => {
        void mutate();
      })
      .catch((error) => {
        void mutate();
        console.error('Failed to delete model:', error);
        message.error(t('settings.saveModelConfigFailed'));
      });
  };

  const removePlatform = (id: ProviderId) => {
    const nextArray = (data ?? []).filter((item: IProvider) => item.id !== id);
    void mutate(nextArray, false);
    ipcBridge.mode.deleteProvider
      .invoke({ provider_id: id })
      .then(() => {
        void mutate();
      })
      .catch((error) => {
        void mutate();
        console.error('Failed to delete provider:', error);
        if (isBackendHttpError(error) && error.code === 'PROVIDER_IN_USE') {
          const groups = groupUsagesByFeature(parseProviderInUseDetails(error.details));
          const featureName: Record<ProviderUsageFeature, string> = {
            desktopCompanion: t('settings.providerInUse.desktopCompanion'),
            customerService: t('settings.providerInUse.customerService'),
            conversation: t('settings.providerInUse.conversation'),
            agentExecution: t('settings.providerInUse.agentExecution'),
          };
          Modal.confirm({
            title: t('settings.providerInUse.title'),
            content: (
              <div className='flex flex-col gap-8px'>
                <div>{t('settings.providerInUse.desc')}</div>
                {groups.map((g) => (
                  <div key={g.feature}>
                    <b>{featureName[g.feature]}</b>：{g.labels.join('、')}
                  </div>
                ))}
              </div>
            ),
            okText: t('settings.providerInUse.goto'),
            cancelText: t('common.cancel', { defaultValue: '取消' }),
            onOk: () => {
              const first = groups[0];
              if (first) navigate(featureRoute(first.feature, first.targetId));
            },
          });
          return;
        }
        message.error(t('settings.saveModelConfigFailed'));
      });
  };

  const persistProviderOrder = async (nextArray: IProvider[], previousArray: IProvider[]) => {
    const previousById = new Map(previousArray.map((item) => [item.id, item.sort_order]));
    const changed = nextArray.filter((item) => previousById.get(item.id) !== item.sort_order);

    if (changed.length === 0) return;

    await Promise.all(
      changed.map((platform) =>
        ipcBridge.mode.updateProvider.invoke({
          provider_id: platform.id,
          sort_order: platform.sort_order,
        })
      )
    );
  };

  const handleProviderDragEnd = (activeData: SortableDragData, overData: SortableDragData) => {
    if (!data || activeData.type !== 'provider' || overData.type !== 'provider') return;

    const reordered = reorderById(editableProviders, activeData.providerId, overData.providerId);
    if (reordered === editableProviders) return;

    // Preserve every managed provider's full-list slot. Refill only editable
    // slots and assign their full-list position as sort_order, avoiding a
    // duplicate sort_order with a managed row (whose CRUD is protected).
    let editableIndex = 0;
    const nextArray = data.map((item, fullIndex) => {
      if (isManagedModelProvider(item)) return item;
      return { ...reordered[editableIndex++], sort_order: fullIndex };
    });
    const reorderedWithOrder = nextArray.filter((item) => !isManagedModelProvider(item));
    void mutate(nextArray, false);

    persistProviderOrder(reorderedWithOrder, editableProviders)
      .then(() => {
        void mutate();
      })
      .catch((error) => {
        void mutate();
        console.error('Failed to save provider order:', error);
        message.error(t('settings.saveModelConfigFailed'));
      });
  };

  /**
   * 模型排序：对受影响的行逐条全量 providerModel.save。
   * Model reorder persists full canonical rows with the new sort order.
   */
  const handleModelDragEnd = (activeData: SortableDragData, overData: SortableDragData) => {
    if (activeData.type !== 'model' || overData.type !== 'model' || activeData.providerId !== overData.providerId) {
      return;
    }

    const platform = (data || []).find((item) => item.id === activeData.providerId);
    if (!platform) return;

    const rows = modelRowsFor(platform);
    const names = rows.map((row) => row.model);
    const nextNames = reorderStrings(names, activeData.model, overData.model);
    if (nextNames === names) return;

    const rowByModel = new Map(rows.map((row) => [row.model, row]));
    const nextRows = nextNames.map((name, index) => ({ ...rowByModel.get(name)!, sort_order: index }));
    const changed = nextRows.filter((row) => rowByModel.get(row.model)!.sort_order !== row.sort_order);

    if (data) {
      const nextArray = data.map((item) =>
        item.id === platform.id
          ? {
              ...item,
              models: nextRows,
            }
          : item
      );
      void mutate(nextArray, false);
    }

    Promise.all(
      changed.map((row) =>
        ipcBridge.providerModel.save.invoke({
          provider_id: platform.id,
          model: providerModelInputFor(row),
        })
      )
    )
      .then(() => {
        void mutate();
      })
      .catch((error) => {
        void mutate();
        console.error('Failed to save model order:', error);
        message.error(t('settings.saveModelConfigFailed'));
      });
  };

  const handleDragEnd = ({ active, over }: DragEndEvent) => {
    if (!over || active.id === over.id) return;

    const activeData = active.data.current as SortableDragData | undefined;
    const overData = over.data.current as SortableDragData | undefined;
    if (!activeData || !overData || activeData.type !== overData.type) return;

    if (activeData.type === 'provider') {
      handleProviderDragEnd(activeData, overData);
    } else {
      handleModelDragEnd(activeData, overData);
    }
  };

  /**
   * 服务端整组克隆：`POST /api/providers/{id}/clone` 连模型档案与连接档案一起复制。
   * 副本名走本地化后缀（`settings.providerCopySuffix`），由 FE 随 body 传给克隆端点。
   * Server-side provider clone (models + connections copied atomically); the
   * copy's display name carries a localized suffix supplied by the frontend.
   */
  const duplicatePlatform = (platform: IProvider) => {
    ipcBridge.mode.cloneProvider
      .invoke({
        provider_id: platform.id,
        name: `${platform.name} ${t('settings.providerCopySuffix', { defaultValue: '副本' })}`,
      })
      .then((copied) => {
        void mutate();
        setCollapseKey((prev) => ({ ...prev, [copied.id]: true }));
        message.success(t('settings.providerConfigCopied', { name: copied.name }));
      })
      .catch((error) => {
        console.error('Failed to clone provider:', error);
        message.error(t('settings.saveModelConfigFailed'));
      });
  };

  // 供应商启用开关只写 provider.enabled，不批量改变各模型的启用状态。
  // Provider enable switch writes provider.enabled itself.
  const toggleProviderEnabled = (platform: IProvider) => {
    const enabled = platform.enabled === false;
    if (data) {
      void mutate(
        data.map((item) => (item.id === platform.id ? { ...item, enabled } : item)),
        false
      );
    }
    ipcBridge.mode.updateProvider
      .invoke({ provider_id: platform.id, enabled })
      .then(() => {
        void mutate();
      })
      .catch((error) => {
        void mutate();
        console.error('Failed to update provider enabled state:', error);
        message.error(t('settings.saveModelConfigFailed'));
      });
  };

  // Execute provider/model health check without creating a conversation.
  const performHealthCheck = async (platform: IProvider, modelName: string, task: ModelTask) => {
    const loadingKey = `${platform.id}-${modelName}`;
    setHealthCheckLoading((prev) => ({ ...prev, [loadingKey]: true }));

    const startTime = Date.now();

    try {
      const result = await ipcBridge.agentConversation.checkProviderHealth.invoke({
        provider_id: platform.id,
        model: modelName,
        task,
      });
      const latency = result.elapsed_ms || Date.now() - startTime;
      const success = result.status === 'healthy';
      const errorMessage = result.message || t('common.unknownError');

      // The server persists the task capability result; refresh the projection.
      await mutate();
      if (success) {
        Message.success({
          content: `${platform.name} - ${modelName}: ${t('common.success')} (${latency}ms)`,
          duration: 3000,
        });
      } else {
        Message.error({
          content: (
            <span data-health-error-kind={result.error_kind} data-health-http-status={result.http_status}>
              {`${platform.name} - ${modelName}: ${healthFailureHeadline(t, result)}`}
              <br />
              {errorMessage}
              {result.attempted_url && (
                <>
                  <br />
                  <span className='text-t-tertiary break-all'>
                    {t('settings.modelAdvanced.resolvedUrl', { defaultValue: '实际请求地址' })}:{' '}
                    {result.attempted_url}
                  </span>
                </>
              )}
            </span>
          ),
          duration: 8000,
        });
      }
    } catch (error: unknown) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      Message.error({
        content: `${platform.name} - ${modelName}: ${t('common.failed')} - ${errorMessage}`,
        duration: 5000,
      });
      // The probe request itself failed (transport error). Refresh anyway in
      // case the server recorded a row-level result before the failure.
      await mutate().catch(() => undefined);
    } finally {
      setHealthCheckLoading((prev) => ({ ...prev, [loadingKey]: false }));
    }
  };

  const [addPlatformModalCtrl, addPlatformModalContext] = AddPlatformModal.useModal({
    onSubmit(platform) {
      setCollapseKey((prev) => ({ ...prev, [platform.id]: true }));
      void mutate();
    },
  });

  // Consume pending deep-link data on mount (set by useDeepLink hook before navigation)
  useEffect(() => {
    const pending = consumePendingDeepLink();
    if (pending) {
      addPlatformModalCtrl.open({ deepLinkData: pending });
    }
  }, [addPlatformModalCtrl]);

  const [addModelModalCtrl, addModelModalContext] = AddModelModal.useModal({
    onSubmit(platform) {
      setCollapseKey((prev) => ({ ...prev, [platform.id]: true }));
      void mutate();
    },
  });

  const [editModalCtrl, editModalContext] = EditModeModal.useModal({
    async onChange(patch) {
      const { id, credentials, ...publicPatch } = patch;
      if (data) {
        void mutate(
          data.map((provider) =>
            provider.id === id
              ? {
                  ...provider,
                  ...publicPatch,
                  has_credentials:
                    credentials === undefined
                      ? provider.has_credentials
                      : Object.keys(credentials).length > 0,
                }
              : provider
          ),
          false
        );
      }
      try {
        await ipcBridge.mode.updateProvider.invoke({
          provider_id: id,
          ...publicPatch,
          ...(credentials === undefined ? {} : { credentials }),
        });
        await mutate();
      } catch (error) {
        void mutate();
        console.error('Failed to save provider:', error);
        message.error(t('settings.saveModelConfigFailed'));
        throw error;
      }
    },
  });

  return (
    <div ref={paneRef} className='flex flex-col'>
      {messageContext}
      {addPlatformModalContext}
      {editModalContext}
      {addModelModalContext}

      {/* Header with Add Button */}
      <div className='flex-shrink-0 border-b border-b-solid border-[var(--color-border-2)] pb-12px mb-14px flex flex-col gap-10px'>
        <ModelHubPageHeader
          title={t('settings.modelHub.provider.title')}
          description={t('settings.modelHub.provider.subtitle')}
          actions={
            <Button
              type='outline'
              shape='round'
              icon={<Plus size='16' />}
              onClick={() => addPlatformModalCtrl.open()}
              className='rd-100px border-1px border-solid border-[var(--color-border-2)] h-34px px-14px text-t-secondary hover:text-t-primary'
            >
              {t('settings.addModel')}
            </Button>
          }
        />
        <div
          className='rd-10px px-12px py-10px border border-solid flex items-start gap-9px'
          style={{
            borderColor: 'rgba(var(--primary-6),0.24)',
            backgroundColor: 'rgba(var(--primary-6),0.06)',
          }}
        >
          <Info theme='outline' size='16' className='mt-1px shrink-0 text-primary-6' />
          <div className='min-w-0'>
            <div className='text-13px font-600 leading-18px text-t-primary'>
              {t('settings.modelHub.provider.noticeTitle')}
            </div>
            <div className='mt-2px text-12px leading-18px text-t-secondary'>
              {t('settings.modelHub.provider.note')}
            </div>
          </div>
        </div>
        {/* 职责收窄：本页只管接入与凭证；「按用途找模型」已迁到模态分区。 */}
        <div className='mt-8px text-12px leading-18px text-t-tertiary'>
          {t('settings.modelHub.provider.scopeNote')}
        </div>
      </div>

      {/* Content Area */}
      <NomiScrollArea className='flex-1 min-h-0' disableOverflow>
        {!data || editableProviders.length === 0 ? (
          <div className='flex flex-col items-center justify-center py-40px'>
            <Info theme='outline' size='48' className='text-t-secondary mb-16px' />
            <h3 className='text-16px font-500 text-t-primary mb-8px'>{t('settings.noConfiguredModels')}</h3>
            <p className='text-14px text-t-secondary text-center max-w-400px'>
              {t('settings.needHelpConfigGuide')}
              <a
                href='https://github.com/nomifun/nomifun-app/wiki/LLM-Configuration'
                target='_blank'
                rel='noopener noreferrer'
                className='text-primary-6 hover:text-primary-5 underline ml-4px'
              >
                {t('settings.configGuide')}
              </a>
              {t('settings.configGuideSuffix')}
            </p>
          </div>
        ) : (
          <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
            <SortableContext items={providerSortableItems} strategy={verticalListSortingStrategy}>
              <div className='space-y-16px'>
            {editableProviders.map((platform: IProvider) => {
              const key = platform.id;
              const isExpanded = collapseKey[platform.id] ?? false;
              const modelRows = modelRowsFor(platform);
              return (
                <SortableProviderCard key={key} provider={platform}>
                  {({ attributes, listeners, setActivatorNodeRef, isDragging }) => (
                <Collapse
                  activeKey={isExpanded ? ['image-generation'] : []}
                  onChange={(_, activeKeys) => {
                    const expanded = activeKeys.includes('image-generation');
                    setCollapseKey((prev) => ({ ...prev, [platform.id]: expanded }));
                  }}
                  bordered
                  expandIconPosition='left'
                  className={`[&_.arco-collapse-item]:!border-0 [&_.arco-collapse-item]:!rounded-12px [&_.arco-collapse-item]:!overflow-hidden [&_.arco-collapse-item]:!bg-[var(--color-bg-2)] [&_.arco-collapse-item-header]:!bg-[var(--fill-0)] [&_.arco-collapse-item-header]:!pl-36px [&_.arco-collapse-item-header]:!pr-12px [&_.arco-collapse-item-header]:!py-8px [&_.arco-collapse-item-header]:transition-colors [&_.arco-collapse-item-header]:hover:!bg-[var(--color-bg-2)] [&_.arco-collapse-item-header]:!gap-8px [&_.arco-collapse-item-header-title]:!min-w-0 [&_.arco-collapse-item-header-icon]:!text-2 [&_.arco-collapse-item-header:hover_.arco-collapse-item-header-icon]:!text-1 [&_.arco-collapse-item-content]:!bg-fill-1 [&_.arco-collapse-item-content-box]:!px-10px [&_.arco-collapse-item-content-box]:!py-8px [&_.arco-collapse-item-content]:!border-t [&_.arco-collapse-item-content]:!border-t-solid [&_.arco-collapse-item-content]:!border-[var(--color-border-2)] ${
                    isExpanded
                      ? '[&_.arco-collapse-item-header]:!rounded-t-12px [&_.arco-collapse-item-header]:!rounded-b-0 [&_.arco-collapse-item-content]:!rounded-b-12px'
                      : '[&_.arco-collapse-item-header]:!rounded-12px'
                  }`}
                >
                  <Collapse.Item
                    name='image-generation'
                    className='[&_.arco-collapse-item-header-title]:flex-1 group'
                    header={
                      <div className='group flex items-center justify-between w-full min-h-32px gap-8px min-w-0'>
                        <div className='flex items-center gap-8px min-w-0 flex-1'>
                          <PriorityDragHandle
                            attributes={attributes}
                            listeners={listeners}
                            setActivatorNodeRef={setActivatorNodeRef}
                            isDragging={isDragging}
                            label={t('settings.dragProviderPriority', { defaultValue: '拖拽调整供应商优先级' })}
                          />
                          <span
                            className={`text-14px font-500 truncate min-w-0 transition-colors ${isExpanded ? 'text-t-primary' : 'text-2 group-hover:text-1'}`}
                          >
                            {platform.name}
                          </span>
                        </div>
                        <div
                          className='flex items-center gap-8px shrink-0'
                          onClick={(e) => {
                            e.stopPropagation();
                          }}
                          onMouseDown={(e) => {
                            e.stopPropagation();
                          }}
                        >
                          <span className={`text-12px text-t-secondary whitespace-nowrap items-center ${isWide ? 'inline-flex' : 'hidden'}`}>
                            <span
                              className='cursor-pointer hover:text-t-primary transition-colors'
                              onClick={() => setCollapseKey((prev) => ({ ...prev, [platform.id]: !isExpanded }))}
                            >
                              {t('settings.modelCount')}（{modelRows.length}）
                            </span>
                            <span className='mx-6px'>|</span>
                            <span
                              className='cursor-pointer hover:text-t-primary transition-colors'
                              onClick={() => editModalCtrl.open({ data: platform })}
                            >
                              {platform.has_credentials
                                ? t('settings.connections.hasCredentials')
                                : t('settings.connections.noCredentials')}
                            </span>
                          </span>
                          <span className={`text-12px text-t-secondary whitespace-nowrap ${isWide ? 'hidden' : 'inline'}`}>
                            {modelRows.length} ·{' '}
                            {platform.has_credentials
                              ? t('settings.connections.hasCredentials')
                              : t('settings.connections.noCredentials')}
                          </span>
                          {/* 供应商启用开关（写 provider.enabled）/ Provider enable switch */}
                          <Switch
                            size='small'
                            checked={platform.enabled !== false}
                            onChange={() => toggleProviderEnabled(platform)}
                          />
                          <div className='flex items-center gap-4px'>
                            <Button
                              size='mini'
                              className='model-provider-action-btn !w-28px !h-28px !min-w-28px text-t-secondary hover:text-t-primary'
                              icon={<Plus size='14' />}
                              onClick={() => addModelModalCtrl.open({ data: platform })}
                            />
                            <Popconfirm
                              title={t('settings.deleteAllModelConfirm')}
                              onOk={() => removePlatform(platform.id)}
                            >
                              <Button
                                size='mini'
                                className='model-provider-action-btn !w-28px !h-28px !min-w-28px text-t-secondary hover:text-t-primary'
                                icon={<Minus size='14' />}
                              />
                            </Popconfirm>
                            <Button
                              size='mini'
                              className='model-provider-action-btn !w-28px !h-28px !min-w-28px text-t-secondary hover:text-t-primary'
                              icon={<Write size='14' />}
                              onClick={() => editModalCtrl.open({ data: platform })}
                            />
                            <Tooltip content={t('settings.copyProviderConfig', { defaultValue: '复制整组配置' })}>
                              <Button
                                size='mini'
                                className='model-provider-action-btn !w-28px !h-28px !min-w-28px text-t-secondary hover:text-t-primary'
                                icon={<Copy theme='outline' size='14' />}
                                onClick={() => duplicatePlatform(platform)}
                              />
                            </Tooltip>
                          </div>
                        </div>
                      </div>
                    }
                  >
                    <SortableContext
                      items={modelRows.map((row) => modelSortableId(platform.id, row.model))}
                      strategy={verticalListSortingStrategy}
                    >
                    {modelRows.map((row: ProviderModelResponse, index: number, arr: ProviderModelResponse[]) => {
                      const model = row.model;
                      const unhealthyCapability = row.capabilities.find(
                        (capability) => capability.health?.status === 'unhealthy'
                      );
                      const checkedCapability =
                        unhealthyCapability ?? row.capabilities.find((capability) => capability.health);
                      const aggregateCapabilityHealth = checkedCapability?.health;
                      const healthStatus = aggregateCapabilityHealth?.status || 'unknown';
                      const modelDescription = row.description ?? '';
                      const healthTasks = row.capabilities.map((capability) => capability.task);

                      return (
                        <SortableModelRow key={model} providerId={platform.id} model={model}>
                          {({
                            attributes: modelAttributes,
                            listeners: modelListeners,
                            setActivatorNodeRef: setModelActivatorNodeRef,
                            isDragging: modelIsDragging,
                          }) => (
                        <div>
                          <div className='flex items-center justify-between px-8px py-12px transition-colors hover:bg-[var(--fill-0)]'>
                            <div className='flex flex-col min-w-0 flex-1 gap-2px'>
                              <div className='flex items-center gap-8px min-w-0'>
                                <PriorityDragHandle
                                  attributes={modelAttributes}
                                  listeners={modelListeners}
                                  setActivatorNodeRef={setModelActivatorNodeRef}
                                  isDragging={modelIsDragging}
                                  label={t('settings.dragModelPriority', { defaultValue: '拖拽调整模型优先级' })}
                                />

                                {/* 健康状态指示器 / Health status indicator */}
                                {healthStatus !== 'unknown' && (
                                  <Tooltip
                                    content={
                                      <div>
                                        <div className='flex items-center gap-4px'>
                                          <span>{healthStatus === 'healthy' ? '✅' : '❌'}</span>
                                          <span>
                                            {healthStatus === 'healthy' ? t('common.success') : t('common.failed')}
                                          </span>
                                        </div>
                                        {checkedCapability && (
                                          <div className='text-12px mt-4px'>
                                            {t('settings.modelModality', { defaultValue: '模态能力' })}:{' '}
                                            {t(`settings.modelTask.${checkedCapability.task}`, {
                                              defaultValue: checkedCapability.task,
                                            })}
                                          </div>
                                        )}
                                        {aggregateCapabilityHealth?.latency !== undefined && (
                                          <div className='text-12px mt-4px'>
                                            {t('settings.latency')}: {aggregateCapabilityHealth.latency}ms
                                          </div>
                                        )}
                                        {aggregateCapabilityHealth?.error && (
                                          <div className='text-12px mt-4px'>{aggregateCapabilityHealth.error}</div>
                                        )}
                                        {checkedCapability?.health_checked_at && (
                                          <div className='text-12px mt-4px'>
                                            {t('mcp.lastCheck')}:{' '}
                                            {new Date(checkedCapability.health_checked_at).toLocaleString()}
                                          </div>
                                        )}
                                      </div>
                                    }
                                  >
                                    <div
                                      className={`w-8px h-8px rounded-full shrink-0 ${healthStatus === 'healthy' ? 'bg-green-500' : 'bg-red-500'}`}
                                    />
                                  </Tooltip>
                                )}

                                <span className='text-14px text-t-primary min-w-0 truncate' title={model}>
                                  {model}
                                </span>

                                {/* Each modality badge reports only that task capability's probe status. */}
                                {row.capabilities.map((capability) => (
                                  <CapabilityHealthTag key={capability.task} capability={capability} />
                                ))}

                                {/* One editor owns modality, protocol, transport and provider params. */}
                                <ModelAdvancedEditor
                                  providerId={platform.id}
                                  preset={platform.platform}
                                  providerBaseUrl={platform.base_url}
                                  providerAuthScheme={platform.auth_scheme}
                                  model={model}
                                  capabilities={row.capabilities}
                                  onSave={(patch) => updateModelCapabilities(platform, row, patch.capabilities)}
                                />

                                {/* 模型启用开关（行级）/ Model enable switch (row-level) */}
                                <Switch
                                  size='small'
                                  className='shrink-0'
                                  checked={row.enabled}
                                  onChange={(checked) => {
                                    void updateModelRow(platform, row, { enabled: checked });
                                  }}
                                />

                                {/* 每模型描述编辑（驱动智能协作选择）/ Per-model collaboration description */}
                                <ModelDescriptionEditor
                                  description={modelDescription}
                                  onSave={(text) => {
                                    void updateModelRow(platform, row, { description: text || null });
                                  }}
                                />
                              </div>

                              {/* 描述次级行（空态隐藏）/ Description secondary line (hidden when empty) */}
                              {modelDescription && (
                                <div
                                  className='text-12px text-t-secondary leading-snug line-clamp-2 break-words pr-8px'
                                  title={modelDescription}
                                >
                                  {modelDescription}
                                </div>
                              )}
                            </div>

                            <div className='flex items-center gap-6px shrink-0'>
                              {/* 单任务一键检测；多任务先选具体模态 / Single task: one click; multi-task: explicit picker. */}
                              <ModelHealthCheckAction
                                tasks={healthTasks}
                                loading={healthCheckLoading[`${platform.id}-${model}`]}
                                onCheck={(task) => performHealthCheck(platform, model, task)}
                              />

                              <Popconfirm
                                title={t('settings.deleteModelConfirm')}
                                onOk={() => removeModel(platform, model)}
                              >
                                <Button
                                  size='mini'
                                  className='!w-28px !h-28px !min-w-28px !bg-[var(--color-bg-1)] text-t-secondary hover:text-t-primary hover:!bg-[var(--fill-0)]'
                                  icon={<DeleteFour theme='outline' size='18' strokeWidth={2} />}
                                />
                              </Popconfirm>
                            </div>
                          </div>
                          {index < arr.length - 1 && <Divider className='!my-0 !border-[var(--color-border-2)]/70' />}
                        </div>
                          )}
                        </SortableModelRow>
                      );
                    })}
                    </SortableContext>

                    {/* 连接档案区 / Per-role connection profiles */}
                    {modelRows.length > 0 && <Divider className='!my-4px !border-[var(--color-border-2)]/70' />}
                    <ProviderConnectionsSection provider={platform} />
                  </Collapse.Item>
                </Collapse>
                  )}
                </SortableProviderCard>
              );
            })}
              </div>
            </SortableContext>
          </DndContext>
        )}
      </NomiScrollArea>
    </div>
  );
};

export default ModelModalContent;
