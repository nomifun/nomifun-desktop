/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IProvider, ModelProfile, ModelTask, ModelTrait } from '@/common/config/storage';
import type { ProviderModelResponse, UpdateProviderModelRequest } from '@/common/types/provider/providerModel';
import type { ProviderId } from '@/common/types/ids';
import { Button, Checkbox, Collapse, Divider, Input, Message, Modal, Popconfirm, Popover, Select, Switch, Tag, Tooltip } from '@arco-design/web-react';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { Copy, DeleteFour, Info, Minus, Plus, Write, Heartbeat, Drag, TagOne } from '@icon-park/react';
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
import { isNewApiPlatform, NEW_API_PROTOCOL_OPTIONS } from '@/renderer/utils/model/modelPlatforms';
import EditModeModal from '@/renderer/pages/settings/components/EditModeModal';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { useModelProfiles } from '@/renderer/hooks/agent/useModelProfiles';
import { useContainerWidth } from '@/renderer/hooks/ui/useContainerWidth';
import { consumePendingDeepLink } from '@/renderer/hooks/system/useDeepLink';
import { ContextLimitSelect, formatContextLimit } from '@/renderer/pages/settings/components/ContextLimitSelect';
import { isManagedModelProvider } from '@/common/types/provider/managedModelService';
import { reorderById, reorderStrings } from './modelProviderOrdering';
import {
  buildModelProfileUpsertRequest,
  editableModelTasks,
  editableModelTraits,
  isInferredModelProfile,
  MODEL_TASK_ORDER,
  MODEL_TRAIT_ORDER,
  primaryModelTask,
  visibleModelTaskBadges,
} from '@/renderer/hooks/agent/modelProfileEditing';
import '../model-provider.css';

/**
 * 获取协议显示标签颜色
 * Get protocol badge color
 */
const getProtocolColor = (protocol: string): string => {
  switch (protocol) {
    case 'gemini':
      return 'blue';
    case 'anthropic':
      return 'orange';
    case 'openai':
    default:
      return 'green';
  }
};

/**
 * 获取协议显示名称
 * Get protocol display name
 */
const getProtocolLabel = (protocol: string): string => {
  return NEW_API_PROTOCOL_OPTIONS.find((p) => p.value === protocol)?.label || 'OpenAI';
};

/**
 * 获取下一个协议（循环切换）
 * Get next protocol (cycle through options)
 */
const getNextProtocol = (current: string): string => {
  const idx = NEW_API_PROTOCOL_OPTIONS.findIndex((p) => p.value === current);
  const nextIdx = (idx + 1) % NEW_API_PROTOCOL_OPTIONS.length;
  return NEW_API_PROTOCOL_OPTIONS[nextIdx].value;
};

// Calculate API Key count
const getApiKeyCount = (api_key: string): number => {
  if (!api_key) return 0;
  return api_key.split(/[,\n]/).filter((k) => k.trim().length > 0).length;
};

/**
 * 权威 per-model 行：优先 `models_detail`（provider_models 投影）；无投影时
 * 从 legacy 字段合成只读行（仅剩托管/异常供应商会走到这里）。
 * Prefer the authoritative `models_detail` rows; synthesize fallback rows
 * from the legacy projection fields when absent.
 */
const modelRowsFor = (platform: IProvider): ProviderModelResponse[] => {
  if (platform.models_detail && platform.models_detail.length > 0) return platform.models_detail;
  return (platform.models ?? []).map((model, index) => ({
    provider_id: platform.id,
    model,
    enabled: platform.model_enabled?.[model] !== false,
    sort_order: index,
    tasks: [],
    traits: [],
    protocol: platform.model_protocols?.[model],
    params: null,
    context_limit: platform.model_context_limits?.[model],
    description: platform.model_descriptions?.[model],
    source: 'inferred',
    health: platform.model_health?.[model],
    created_at: 0,
    updated_at: 0,
  }));
};

/** Row-level partial update body minus the composite key. */
type ModelRowPatch = Omit<UpdateProviderModelRequest, 'provider_id' | 'model'>;

/** Apply a tri-state row patch to the cached row for optimistic rendering. */
const applyRowPatch = (row: ProviderModelResponse, patch: ModelRowPatch): ProviderModelResponse => ({
  ...row,
  ...(patch.enabled !== undefined ? { enabled: patch.enabled } : {}),
  ...(patch.sort_order !== undefined ? { sort_order: patch.sort_order } : {}),
  ...(patch.tasks !== undefined ? { tasks: patch.tasks } : {}),
  ...(patch.traits !== undefined ? { traits: patch.traits } : {}),
  ...(patch.protocol !== undefined ? { protocol: patch.protocol ?? undefined } : {}),
  ...(patch.connection_role !== undefined ? { connection_role: patch.connection_role ?? undefined } : {}),
  ...(patch.params !== undefined ? { params: patch.params } : {}),
  ...(patch.context_limit !== undefined ? { context_limit: patch.context_limit ?? undefined } : {}),
  ...(patch.description !== undefined ? { description: patch.description ?? undefined } : {}),
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

/**
 * 每模型上下文窗口编辑浮层 / Per-model context window editor popover.
 */
const ModelContextLimitEditor: React.FC<{
  value?: number;
  onSave: (value?: number) => void;
}> = ({ value, onSave }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<number | undefined>(value);

  const handleVisibleChange = (visible: boolean) => {
    if (visible) setDraft(value);
    setOpen(visible);
  };

  const handleSave = () => {
    onSave(draft);
    setOpen(false);
  };

  const label = value
    ? formatContextLimit(value)
    : t('settings.modelContextLimitDefault', { defaultValue: '默认' });

  return (
    <Popover
      trigger='click'
      position='bl'
      popupVisible={open}
      onVisibleChange={handleVisibleChange}
      content={
        <div className='flex flex-col gap-8px w-240px' onClick={(e) => e.stopPropagation()}>
          <div className='text-12px text-t-secondary'>
            {t('settings.modelContextLimit', { defaultValue: '模型上下文窗口' })}
          </div>
          <ContextLimitSelect value={draft} onChange={setDraft} />
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
      <Tooltip content={t('settings.editModelContextLimit', { defaultValue: '编辑模型上下文窗口' })}>
        <Button
          size='mini'
          className={`model-provider-action-btn !h-24px !min-w-44px shrink-0 px-6px text-11px ${value ? 'text-primary-6 hover:text-primary-5' : 'text-t-secondary hover:text-t-primary'}`}
          onClick={(e) => e.stopPropagation()}
        >
          {label}
        </Button>
      </Tooltip>
    </Popover>
  );
};

/**
 * 模态能力编辑浮层。推断档案（source='inferred'）的任务/能力预勾选展示，
 * 并带「系统推断」提示标；保存即转为 user 档案。四项 trait 全部可编辑。
 * Inferred profiles are shown pre-checked with a hint tag; saving converts
 * them to a user profile. All four traits are editable.
 */
const ModelModalityEditor: React.FC<{
  profile?: ModelProfile;
  onSave: (tasks: ModelTask[], traits: ModelTrait[]) => Promise<void>;
}> = ({ profile, onSave }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [draftTasks, setDraftTasks] = useState<ModelTask[]>(() => editableModelTasks(profile));
  const [draftTraits, setDraftTraits] = useState<ModelTrait[]>(() => editableModelTraits(profile));
  const isInferred = isInferredModelProfile(profile);
  const taskOptions = useMemo(
    () => MODEL_TASK_ORDER.map((v) => ({ label: t(`settings.modelTask.${v}`), value: v })),
    [t]
  );
  const hasUserSelection =
    profile?.source === 'user' && ((profile.tasks?.length ?? 0) > 0 || (profile.traits?.length ?? 0) > 0);

  const handleVisibleChange = (visible: boolean) => {
    if (visible) {
      setDraftTasks(editableModelTasks(profile));
      setDraftTraits(editableModelTraits(profile));
    }
    setOpen(visible);
  };

  const toggleTrait = (trait: ModelTrait, checked: boolean) => {
    setDraftTraits((prev) => (checked ? [...prev.filter((item) => item !== trait), trait] : prev.filter((item) => item !== trait)));
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      // Persist traits in canonical display order for stable badges/serialization.
      await onSave(draftTasks, MODEL_TRAIT_ORDER.filter((trait) => draftTraits.includes(trait)));
      setOpen(false);
    } catch {
      // Parent save handler owns the toast; keep the editor open so the user can retry.
    } finally {
      setSaving(false);
    }
  };

  return (
    <Popover
      trigger='click'
      position='bl'
      popupVisible={open}
      onVisibleChange={handleVisibleChange}
      content={
        <div className='flex flex-col gap-8px w-280px' onClick={(e) => e.stopPropagation()}>
          <div className='flex items-center gap-6px text-12px text-t-secondary'>
            <span>{t('settings.modelModality', { defaultValue: '模态能力' })}</span>
            {isInferred && (
              <Tag size='small' color='arcoblue' bordered className='select-none'>
                {t('settings.modelInferredTag', { defaultValue: '系统推断' })}
              </Tag>
            )}
          </div>
          {isInferred && (
            <div className='text-11px text-t-tertiary leading-4'>
              {t('settings.modelInferredHint', {
                defaultValue: '以下为系统推断的能力，保存后转为人工确认',
              })}
            </div>
          )}
          <Select
            mode='multiple'
            value={draftTasks}
            onChange={(value: ModelTask[]) => setDraftTasks(value ?? [])}
            options={taskOptions}
            placeholder={t('settings.modelModality', { defaultValue: '模态能力' })}
            triggerProps={{ getPopupContainer: () => document.body }}
          />
          <div className='text-11px text-t-tertiary'>
            {t('settings.modelTraitsLabel', { defaultValue: '能力细化（traits）' })}
          </div>
          <div className='flex flex-col gap-2px'>
            {MODEL_TRAIT_ORDER.map((trait) => (
              <Checkbox
                key={trait}
                checked={draftTraits.includes(trait)}
                onChange={(checked) => toggleTrait(trait, checked)}
                className='!pl-0'
              >
                <span className='text-12px text-t-secondary'>{t(`settings.modelTrait.${trait}`)}</span>
              </Checkbox>
            ))}
          </div>
          <div className='text-11px text-t-tertiary leading-4'>
            {t('settings.modelModalityTip', {
              defaultValue: '声明该模型能做什么——探测与调用据此选择正确的端点',
            })}
          </div>
          <div className='flex items-center justify-end gap-8px'>
            <Button size='mini' onClick={() => setOpen(false)} disabled={saving}>
              {t('common.cancel', { defaultValue: '取消' })}
            </Button>
            <Button size='mini' type='primary' loading={saving} onClick={handleSave}>
              {t('common.save', { defaultValue: '保存' })}
            </Button>
          </div>
        </div>
      }
    >
      <Tooltip content={t('settings.editModelModality', { defaultValue: '编辑模型类别' })}>
        <Button
          size='mini'
          className={`model-provider-action-btn !w-24px !h-24px !min-w-24px shrink-0 ${hasUserSelection ? 'text-primary-6 hover:text-primary-5' : 'text-t-secondary hover:text-t-primary'}`}
          icon={<TagOne theme='outline' size='14' />}
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
  const { profileFor, mutate: mutateProfiles } = useModelProfiles();
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
   * Create when the provider id is new, update otherwise.
   * The caller is expected to have mutated the id-bearing record already.
   */
  const persistPlatform = async (platform: IProvider): Promise<void> => {
    const existing = (data || []).some((item) => item.id === platform.id);
    if (existing) {
      const { id, ...body } = platform;
      await ipcBridge.mode.updateProvider.invoke({ provider_id: id, ...body });
    } else {
      await ipcBridge.mode.createProvider.invoke(platform);
    }
  };

  const updatePlatform = (platform: IProvider, success: () => void, throwOnError = false): Promise<void> => {
    const existing = (data || []).find((item) => item.id === platform.id);
    const nextArray = existing
      ? (data || []).map((item) => (item.id === platform.id ? { ...item, ...platform } : item))
      : [...(data || []), platform];

    // Optimistic update
    void mutate(nextArray, false);

    return persistPlatform(platform)
      .then(() => {
        void mutate();
        success();
      })
      .catch((error) => {
        void mutate();
        console.error('Failed to save provider:', error);
        // 409 Conflict — duplicate id (rare pre-launch); different toast
        const msg = error instanceof Error ? error.message : String(error);
        if (msg.includes('409')) {
          message.error(t('settings.providerIdConflict', { defaultValue: 'Provider id already exists, retry.' }));
        } else {
          message.error(t('settings.saveModelConfigFailed'));
        }
        if (throwOnError) throw error;
      });
  };

  /**
   * 行级模型更新（providerModel.update）：修复整 map PUT 的读改写竞态。
   * Row-level partial update with optimistic `models_detail` patch.
   */
  const updateModelRow = (platform: IProvider, model: string, patch: ModelRowPatch, rethrow = false): Promise<void> => {
    if (data) {
      const nextArray = data.map((item) =>
        item.id === platform.id && item.models_detail
          ? {
              ...item,
              models_detail: item.models_detail.map((row) => (row.model === model ? applyRowPatch(row, patch) : row)),
            }
          : item
      );
      void mutate(nextArray, false);
    }
    return ipcBridge.providerModel.update
      .invoke({ provider_id: platform.id, model, ...patch })
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

  /** 行级删除：不再手动清理 5 个 legacy map。 */
  const removeModel = (platform: IProvider, model: string) => {
    if (data) {
      const nextArray = data.map((item) =>
        item.id === platform.id
          ? {
              ...item,
              models: (item.models ?? []).filter((m) => m !== model),
              ...(item.models_detail ? { models_detail: item.models_detail.filter((row) => row.model !== model) } : {}),
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
            smartDecision: t('settings.providerInUse.smartDecision'),
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
   * 模型排序：对受影响的行逐条 providerModel.update({sort_order})。
   * Model reorder persists per-row sort_order updates.
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
              models: nextNames,
              ...(item.models_detail ? { models_detail: nextRows } : {}),
            }
          : item
      );
      void mutate(nextArray, false);
    }

    Promise.all(
      changed.map((row) =>
        ipcBridge.providerModel.update.invoke({
          provider_id: platform.id,
          model: row.model,
          sort_order: row.sort_order,
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

  // 供应商启用开关写 provider.enabled 本身（语义修正：不再批量翻转 model_enabled）
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
  const performHealthCheck = async (platform: IProvider, modelName: string, task?: ModelTask) => {
    const loadingKey = `${platform.id}-${modelName}`;
    setHealthCheckLoading((prev) => ({ ...prev, [loadingKey]: true }));

    const startTime = Date.now();

    try {
      const result = await ipcBridge.acpConversation.checkProviderHealth.invoke({
        provider_id: platform.id,
        model: modelName,
        task,
      });
      const latency = result.elapsed_ms || Date.now() - startTime;
      const success = result.status === 'healthy';
      const errorMessage = result.message || t('common.unknownError');

      // 服务端探针已把结果持久化到 provider_models 行；这里只刷新读投影，
      // 不再 fetch-latest-then-merge 整个 model_health map 回写（冗余写已删除）。
      // The server-side probe persists the result into the model's catalog row;
      // refresh the projection instead of PUTting the legacy health map back.
      await mutate();
      if (success) {
        Message.success({
          content: `${platform.name} - ${modelName}: ${t('common.success')} (${latency}ms)`,
          duration: 3000,
        });
      } else {
        Message.error({
          content: `${platform.name} - ${modelName}: ${t('common.failed')} - ${errorMessage}`,
          duration: 5000,
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
    async onSubmit(platform) {
      await updatePlatform(platform, () => {
        setCollapseKey((prev) => ({ ...prev, [platform.id]: true }));
      }, true);
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
      updatePlatform(platform, () => {
        setCollapseKey((prev) => ({ ...prev, [platform.id]: true }));
        addModelModalCtrl.close();
      });
    },
  });

  const [editModalCtrl, editModalContext] = EditModeModal.useModal({
    onChange(platform) {
      updatePlatform(platform, () => editModalCtrl.close());
    },
  });

  return (
    <div
      ref={paneRef}
      className={`flex flex-col bg-2 rd-16px py-16px ${isWide ? 'px-24px' : 'px-16px'}`}
    >
      {messageContext}
      {addPlatformModalContext}
      {editModalContext}
      {addModelModalContext}

      {/* Header with Add Button */}
      <div className='flex-shrink-0 border-b border-b-solid border-[var(--color-border-2)] pb-12px mb-14px flex flex-col gap-10px'>
        <div className='flex items-center justify-between gap-8px flex-wrap'>
          <div className='min-w-0'>
            <div className='text-20px font-600 text-t-primary leading-28px'>
              {t('settings.modelHub.provider.title')}
            </div>
            <div className='mt-2px text-13px leading-18px text-t-secondary'>
              {t('settings.modelHub.provider.subtitle')}
            </div>
          </div>
          <div className='flex items-center gap-8px flex-wrap'>
            <Button
              type='outline'
              shape='round'
              icon={<Plus size='16' />}
              onClick={() => addPlatformModalCtrl.open()}
              className='rd-100px border-1 border-solid border-[var(--color-border-2)] h-34px px-14px text-t-secondary hover:text-t-primary'
            >
              {t('settings.addModel')}
            </Button>
          </div>
        </div>
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
              const hasDetail = Boolean(platform.models_detail && platform.models_detail.length > 0);
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
                              {t('settings.apiKeyCount')}（{getApiKeyCount(platform.api_key)}）
                            </span>
                          </span>
                          <span className={`text-12px text-t-secondary whitespace-nowrap ${isWide ? 'hidden' : 'inline'}`}>
                            {modelRows.length} / {getApiKeyCount(platform.api_key)}
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
                      const isNewApiProvider = isNewApiPlatform(platform.platform);
                      const modelProtocol = row.protocol || 'openai';
                      // 行 health 即权威（legacy 合成行已在 modelRowsFor 里回填），
                      // 不再直接读 provider.model_health map。
                      const model_health = row.health;
                      const healthStatus = model_health?.status || 'unknown';
                      const modelDescription = row.description ?? '';
                      const modelContextLimit = row.context_limit;
                      // Prefer the authoritative row's profile; fall back to the
                      // model-profiles store when the provider has no rows yet.
                      const modelProfile: ModelProfile | undefined = hasDetail
                        ? {
                            provider_id: platform.id,
                            model,
                            tasks: row.tasks,
                            traits: row.traits,
                            source: row.source,
                            updated_at: row.updated_at,
                          }
                        : profileFor(platform.id, model);

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
                                        {model_health?.latency && (
                                          <div className='text-12px mt-4px'>
                                            {t('settings.latency')}: {model_health.latency}ms
                                          </div>
                                        )}
                                        {model_health?.error && (
                                          <div className='text-12px mt-4px'>{model_health.error}</div>
                                        )}
                                        {model_health?.last_check && (
                                          <div className='text-12px mt-4px'>
                                            {t('mcp.lastCheck')}: {new Date(model_health.last_check).toLocaleString()}
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

                                {/* 模态徽章 / Modality badges — all tasks incl. chat (chat neutral, others colored). */}
                                {visibleModelTaskBadges(modelProfile).map((tk) => (
                                    <Tag
                                      key={tk}
                                      size='small'
                                      color={tk === 'chat' ? 'gray' : 'purple'}
                                      bordered
                                      className='shrink-0 select-none'
                                    >
                                      {t(`settings.modelTask.${tk}`)}
                                    </Tag>
                                  ))}

                                {/* New API 协议标签（点击循环切换，行级写入）/ New API protocol badge (click to cycle, row-level write) */}
                                {isNewApiProvider && (
                                  <Tag
                                    size='small'
                                    color={getProtocolColor(modelProtocol)}
                                    className='cursor-pointer select-none shrink-0'
                                    onClick={() => {
                                      void updateModelRow(platform, model, { protocol: getNextProtocol(modelProtocol) });
                                    }}
                                  >
                                    {getProtocolLabel(modelProtocol)}
                                  </Tag>
                                )}

                                {/* 每模型上下文窗口 / Per-model context window */}
                                <ModelContextLimitEditor
                                  value={modelContextLimit}
                                  onSave={(value) => {
                                    void updateModelRow(platform, model, {
                                      context_limit: value && value > 0 ? value : null,
                                    });
                                  }}
                                />

                                {/* 每模型类别编辑 / Per-model modality editor */}
                                <ModelModalityEditor
                                  profile={modelProfile}
                                  onSave={async (tasks, traits) => {
                                    try {
                                      await ipcBridge.modelProfile.upsert.invoke(
                                        buildModelProfileUpsertRequest(platform.id, model, tasks, traits)
                                      );
                                      await Promise.all([mutateProfiles(), mutate()]);
                                    } catch (error) {
                                      console.error('model profile upsert failed', error);
                                      message.error(t('settings.saveModelConfigFailed'));
                                      throw error;
                                    }
                                  }}
                                />

                                {/* 高级：协议/连接档案/params / Advanced: protocol, connection_role, params */}
                                <ModelAdvancedEditor
                                  providerId={platform.id}
                                  protocol={row.protocol}
                                  connectionRole={row.connection_role}
                                  params={row.params}
                                  onSave={(patch) => updateModelRow(platform, model, patch, true)}
                                />

                                {/* 模型启用开关（行级）/ Model enable switch (row-level) */}
                                <Switch
                                  size='small'
                                  className='shrink-0'
                                  checked={row.enabled}
                                  onChange={(checked) => {
                                    void updateModelRow(platform, model, { enabled: checked });
                                  }}
                                />

                                {/* 每模型描述编辑（驱动智能协作选择）/ Per-model collaboration description */}
                                <ModelDescriptionEditor
                                  description={modelDescription}
                                  onSave={(text) => {
                                    void updateModelRow(platform, model, { description: text || null });
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
                              {/* 心跳检测按钮（携带档案主任务）/ Health check button (probes the profile's primary task) */}
                              <Tooltip content={t('settings.healthCheck')}>
                                <Button
                                  size='mini'
                                  className='!w-28px !h-28px !min-w-28px !bg-[var(--color-bg-1)] text-t-secondary hover:text-t-primary hover:!bg-[var(--fill-0)]'
                                  icon={<Heartbeat theme='outline' size='16' />}
                                  loading={healthCheckLoading[`${platform.id}-${model}`]}
                                  onClick={() => performHealthCheck(platform, model, primaryModelTask(modelProfile))}
                                />
                              </Tooltip>

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
