/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { MODEL_TASK_ORDER, MODEL_TRAIT_ORDER } from '@/common/modelCapabilities';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ModelTrait } from '@/common/protocolBindings/ModelTrait';
import { ttsSupportsProviderParamVoice, ttsVoiceOptionsFor } from '@/renderer/components/model/ttsVoiceOptions';
import { AutoComplete, Button, Checkbox, Input, Popconfirm, Select, Tag, Tooltip } from '@arco-design/web-react';
import { DeleteFour, Down, Refresh, Right } from '@icon-park/react';
import React, { useEffect, useId, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ContextLimitSelect } from './ContextLimitSelect';
import { OutputLimitInput } from './OutputLimitInput';
import {
  compactCapabilityUrlSummary,
  createCapabilityDisclosureState,
  getSettledCapabilityValidationErrors,
  syncCapabilityDisclosureState,
  toggleCapabilityDisclosure,
} from './modelCapabilityDisclosure';
import {
  AUTH_SCHEME_PRESETS,
  buildConnectionCredentials,
  credentialsKindForScheme,
  isValidConnectionRole,
  type ConnectionCredentialsDraft,
} from './providerConnectionForm';
import {
  CAPABILITY_ENDPOINT_FIELDS,
  addCapabilityTask,
  applyCatalogSuggestionForTask,
  catalogSuggestionsForTask,
  changePrimaryModelTask,
  changeCapabilityProtocol,
  effectiveBaseUrl,
  endpointDescriptorValue,
  isCapabilityEndpointField,
  isDuplicateModelId,
  isProtocolAuthSchemeAllowed,
  parseProviderParams,
  providerParamChainRounds,
  protocolDescriptorForDraft,
  providerParamVoice,
  reconcileCapabilityRecommendations,
  removeCapabilityTask,
  resolveModelInputChange,
  requiresCrossOriginConsent,
  resolvedCapabilityUrl,
  rootMatchesShape,
  withProviderParamVoice,
  withProviderParamChainRounds,
  type CapabilityEndpointDescriptor,
  type CapabilityEndpointField,
  type CapabilityValidationResult,
  type ModelCapabilityDraft,
  type ModelDefinitionDraft,
  type ModelProtocolManifestMap,
  type ProviderConnectionDescriptor,
  type ProviderConnectionInput,
} from './providerModelAdvanced';

export interface ModelCatalogSuggestion {
  value: string;
  label: string;
  tasks: ModelTask[];
  traits: ModelTrait[];
}

export interface ModelDefinitionEditorProps {
  value: ModelDefinitionDraft;
  onChange: (value: ModelDefinitionDraft) => void;
  providerBaseUrl: string;
  providerAuthScheme: string;
  manifests: ModelProtocolManifestMap;
  manifestLoadingTasks?: readonly ModelTask[];
  manifestErrorTasks?: readonly ModelTask[];
  validationErrors: CapabilityValidationResult['errors'];
  validationPending?: boolean;
  existingModelIds?: readonly string[];
  modelReadOnly?: boolean;
  catalogSuggestions?: readonly ModelCatalogSuggestion[];
  catalogLoading?: boolean;
  catalogError?: string;
  onRefreshCatalog?: () => void;
  connections?: readonly ProviderConnectionDescriptor[];
  onCreateConnection?: (connection: ProviderConnectionInput) => Promise<void>;
}

const EMPTY_CONNECTION_CREDENTIALS: ConnectionCredentialsDraft = {
  apiKeysText: '',
  appKey: '',
  accessKey: '',
  resourceId: '',
  rawJson: '',
};

const draftKeyForEndpoint = (
  field: CapabilityEndpointField
): 'endpoint' | 'pollEndpoint' | 'contentEndpoint' | 'realtimeEndpoint' => {
  switch (field) {
    case 'endpoint':
      return 'endpoint';
    case 'poll_endpoint':
      return 'pollEndpoint';
    case 'content_endpoint':
      return 'contentEndpoint';
    case 'realtime_endpoint':
      return 'realtimeEndpoint';
  }
};

const storedEndpointFields = (capability: ModelCapabilityDraft): CapabilityEndpointField[] =>
  CAPABILITY_ENDPOINT_FIELDS.filter((field) => Boolean(capability[draftKeyForEndpoint(field)].trim()));

const endpointLabel = (descriptor: CapabilityEndpointDescriptor, task: ModelTask): string => {
  if (descriptor.purpose === 'content') return 'Content endpoint';
  if (descriptor.purpose === 'poll') return 'Poll endpoint';
  if (descriptor.purpose === 'session') return 'Realtime endpoint';
  if (descriptor.field === 'realtime_endpoint' || task === 'realtime_conversation') return 'Realtime endpoint';
  if (descriptor.field === 'poll_endpoint') return 'Poll endpoint';
  if (descriptor.field === 'content_endpoint') return 'Content endpoint';
  return 'Endpoint';
};

const CUSTOM_AUTH_SCHEME = '__custom__';
const InlineConnectionEditor: React.FC<{
  role?: string;
  roleReadOnly?: boolean;
  label?: string;
  baseUrl: string;
  authScheme: string;
  authSchemes: readonly string[];
  requiresCredentials: boolean;
  onSave: (connection: ProviderConnectionInput) => Promise<void>;
}> = ({
  role = '',
  roleReadOnly = false,
  label,
  baseUrl,
  authScheme,
  authSchemes,
  requiresCredentials,
  onSave,
}) => {
  const { t } = useTranslation();
  const options = [...new Set([...authSchemes, authScheme, ...AUTH_SCHEME_PRESETS])].filter(Boolean);
  const initialPreset = options.includes(authScheme) ? authScheme : CUSTOM_AUTH_SCHEME;
  const [connectionRole, setConnectionRole] = useState(role);
  const [connectionLabel, setConnectionLabel] = useState(label ?? '');
  const [connectionBaseUrl, setConnectionBaseUrl] = useState(baseUrl);
  const [schemeSelection, setSchemeSelection] = useState(initialPreset);
  const [customScheme, setCustomScheme] = useState(initialPreset === CUSTOM_AUTH_SCHEME ? authScheme : '');
  const [credentials, setCredentials] = useState<ConnectionCredentialsDraft>(EMPTY_CONNECTION_CREDENTIALS);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const scheme = schemeSelection === CUSTOM_AUTH_SCHEME ? customScheme.trim() : schemeSelection;
  const credentialsKind = credentialsKindForScheme(scheme);

  const save = async () => {
    if (!isValidConnectionRole(connectionRole) || !connectionBaseUrl.trim() || !scheme) {
      setError(t('settings.connections.completeRequired', { defaultValue: '请完整填写连接角色、地址和鉴权方式。' }));
      return;
    }
    const built = buildConnectionCredentials(scheme, credentials);
    if (!built.ok || (requiresCredentials && built.credentials === undefined)) {
      setError(
        t('settings.connections.credentialsRequired', {
          defaultValue: '该连接需要独立凭据，请完整填写后再创建。',
        })
      );
      return;
    }
    setSaving(true);
    setError('');
    try {
      await onSave({
        role: connectionRole,
        ...(connectionLabel.trim() ? { label: connectionLabel.trim() } : {}),
        base_url: connectionBaseUrl.trim(),
        auth_scheme: scheme,
        // Inline editors always create a new row. The aggregate/create DTO
        // requires an explicit payload; credentialless schemes use `{}`.
        credentials: built.credentials ?? {},
      });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className='rounded-8px border border-solid border-warning-4 bg-warning-1 p-10px space-y-8px'>
      <div className='text-12px font-600 text-warning-7'>
        {roleReadOnly
          ? t('settings.connections.recommendedMissing', {
              defaultValue: `协议需要连接角色 ${connectionRole}，请先创建该连接。`,
              role: connectionRole,
            })
          : t('settings.connections.createCustomTitle', {
              defaultValue: '新建命名连接',
            })}
      </div>
      <Input
        value={connectionRole}
        readOnly={roleReadOnly}
        status={!isValidConnectionRole(connectionRole) ? 'error' : undefined}
        onChange={setConnectionRole}
        placeholder='media / voice / custom_api'
        aria-label={t('settings.connections.role', { defaultValue: '连接角色' })}
      />
      <Input
        value={connectionLabel}
        onChange={setConnectionLabel}
        placeholder={t('settings.connections.label', { defaultValue: '连接名称' })}
      />
      <Input
        value={connectionBaseUrl}
        status={!connectionBaseUrl.trim() ? 'error' : undefined}
        onChange={setConnectionBaseUrl}
        placeholder={t('settings.connections.baseUrl', { defaultValue: '连接 Base URL' })}
      />
      <Select
        value={schemeSelection}
        options={[
          ...options.map((value) => ({ label: value, value })),
          {
            label: t('settings.connections.authSchemeCustom', { defaultValue: '手填已注册格式' }),
            value: CUSTOM_AUTH_SCHEME,
          },
        ]}
        onChange={setSchemeSelection}
        triggerProps={{ getPopupContainer: () => document.body }}
      />
      {schemeSelection === CUSTOM_AUTH_SCHEME && (
        <Input
          value={customScheme}
          onChange={setCustomScheme}
          placeholder='header_key:x-api-key'
        />
      )}
      {credentialsKind === 'api_keys' && (
        <Input.TextArea
          value={credentials.apiKeysText}
          onChange={(apiKeysText) => setCredentials((previous) => ({ ...previous, apiKeysText }))}
          placeholder={t('settings.connections.apiKeys', { defaultValue: 'API Key（多个用逗号或换行分隔）' })}
          rows={3}
        />
      )}
      {credentialsKind === 'volc_voice' && (
        <div className='flex flex-col gap-6px'>
          <Input
            value={credentials.appKey}
            onChange={(appKey) => setCredentials((previous) => ({ ...previous, appKey }))}
            placeholder={t('settings.connections.volcAppKey', { defaultValue: 'App Key' })}
          />
          <Input
            value={credentials.accessKey}
            onChange={(accessKey) => setCredentials((previous) => ({ ...previous, accessKey }))}
            placeholder={t('settings.connections.volcAccessKey', { defaultValue: 'Access Key' })}
          />
          <Input
            value={credentials.resourceId}
            onChange={(resourceId) => setCredentials((previous) => ({ ...previous, resourceId }))}
            placeholder={t('settings.connections.volcResourceId', { defaultValue: 'Resource ID' })}
          />
        </div>
      )}
      {credentialsKind === 'custom' && (
        <Input.TextArea
          value={credentials.rawJson}
          onChange={(rawJson) => setCredentials((previous) => ({ ...previous, rawJson }))}
          placeholder={t('settings.connections.rawCredentials', { defaultValue: '凭据 JSON' })}
          rows={4}
        />
      )}
      {error && <div className='text-11px text-danger-6'>{error}</div>}
      <Button type='primary' size='small' loading={saving} onClick={() => void save()}>
        {t('settings.connections.createInline', { defaultValue: '创建连接并继续配置' })}
      </Button>
    </div>
  );
};

const sameCapabilities = (
  left: readonly ModelCapabilityDraft[],
  right: readonly ModelCapabilityDraft[]
): boolean =>
  left.length === right.length &&
  left.every((capability, index) => {
    const candidate = right[index];
    return (
      candidate?.task === capability.task &&
      candidate.protocol === capability.protocol &&
      candidate.connectionRole === capability.connectionRole &&
      candidate.baseUrlOverride === capability.baseUrlOverride
    );
  });

/**
 * Shared provider-model editor used by create-provider, add-model, and edit-model.
 * Catalog data is advisory. New models choose one primary type before selecting
 * or typing a model ID; existing multi-task models keep their full capability set.
 */
const ModelDefinitionEditor: React.FC<ModelDefinitionEditorProps> = ({
  value,
  onChange,
  providerBaseUrl,
  providerAuthScheme,
  manifests,
  manifestLoadingTasks = [],
  manifestErrorTasks = [],
  validationErrors,
  validationPending = false,
  existingModelIds = [],
  modelReadOnly = false,
  catalogSuggestions = [],
  catalogLoading = false,
  catalogError,
  onRefreshCatalog,
  connections = [],
  onCreateConnection,
}) => {
  const { t } = useTranslation();
  const modelInputId = useId();
  const taskSectionId = useId();
  const capabilityDetailsId = useId();
  const [customConnectionTask, setCustomConnectionTask] = useState<ModelTask>();
  const selectedTasks = useMemo(
    () => value.capabilities.map((capability) => capability.task),
    [value.capabilities]
  );
  const availableTasks = useMemo(
    () => MODEL_TASK_ORDER.filter((task) => !selectedTasks.includes(task)),
    [selectedTasks]
  );
  const primaryTask = modelReadOnly ? undefined : selectedTasks[0];
  const filteredCatalogSuggestions = useMemo(
    () => catalogSuggestionsForTask(catalogSuggestions, primaryTask),
    [catalogSuggestions, primaryTask]
  );
  const recommendationManifests = useMemo(() => {
    const ready: ModelProtocolManifestMap = {};
    for (const task of MODEL_TASK_ORDER) {
      if (!manifestLoadingTasks.includes(task) && manifests[task]) ready[task] = manifests[task];
    }
    return ready;
  }, [manifestLoadingTasks, manifests]);
  const reconciledCapabilities = useMemo(
    () => reconcileCapabilityRecommendations(value.capabilities, recommendationManifests),
    [recommendationManifests, value.capabilities]
  );
  const recommendationPendingTasks = useMemo(
    () =>
      new Set(
        reconciledCapabilities
          .filter((capability, index) => {
            const current = value.capabilities[index];
            return !current || !sameCapabilities([capability], [current]);
          })
          .map((capability) => capability.task)
      ),
    [reconciledCapabilities, value.capabilities]
  );
  const recommendationPending = recommendationPendingTasks.size > 0;
  const settledValidationErrors = useMemo(
    () =>
      getSettledCapabilityValidationErrors(
        validationErrors,
        recommendationPendingTasks,
        validationPending
      ),
    [recommendationPendingTasks, validationErrors, validationPending]
  );
  const [disclosureState, setDisclosureState] = useState(() =>
    createCapabilityDisclosureState(selectedTasks, settledValidationErrors)
  );

  useEffect(() => {
    if (recommendationPending) {
      onChange({ ...value, capabilities: reconciledCapabilities });
    }
  }, [onChange, recommendationPending, reconciledCapabilities, value]);

  useEffect(() => {
    setDisclosureState((current) =>
      syncCapabilityDisclosureState(current, selectedTasks, settledValidationErrors)
    );
  }, [selectedTasks, settledValidationErrors]);

  const duplicateModel = isDuplicateModelId(value.model, existingModelIds);

  const updateCapability = (task: ModelTask, patch: Partial<ModelCapabilityDraft>) => {
    onChange({
      ...value,
      capabilities: value.capabilities.map((capability) =>
        capability.task === task ? { ...capability, ...patch, task } : capability
      ),
    });
  };

  const selectPrimaryTask = (task: ModelTask) => {
    if (task === primaryTask) return;
    onChange(changePrimaryModelTask(value, task));
  };

  const selectCatalogSuggestion = (profile: ModelCatalogSuggestion) => {
    if (!primaryTask) return;
    onChange(
      applyCatalogSuggestionForTask(
        value,
        { model: profile.value, tasks: profile.tasks, traits: profile.traits },
        primaryTask
      )
    );
  };

  const addTask = (task: ModelTask) => {
    onChange({
      ...value,
      capabilities: addCapabilityTask(value.capabilities, task),
    });
  };

  const removeTask = (task: ModelTask) => {
    onChange({
      ...value,
      capabilities: removeCapabilityTask(value.capabilities, task),
    });
  };

  return (
    <div className='flex flex-col gap-16px' data-model-definition-editor>
      {!modelReadOnly ? (
        <>
          <div className='space-y-8px' data-primary-model-task-section>
            <label className='text-13px font-500 text-t-secondary'>
              {t('settings.modelType', { defaultValue: '模型类型' })}
            </label>
            <Select
              value={primaryTask}
              options={MODEL_TASK_ORDER.map((task) => ({
                value: task,
                label: t(`settings.modelTask.${task}`, { defaultValue: task }),
              }))}
              placeholder={t('settings.selectModelType', {
                defaultValue: '先选择模型主要用于什么任务',
              })}
              onChange={(task) => {
                if (typeof task === 'string') selectPrimaryTask(task as ModelTask);
              }}
              triggerProps={{ getPopupContainer: () => document.body }}
              data-primary-model-task-picker
            />
            <div className='text-11px leading-4 text-t-secondary'>
              {t('settings.modelTypeHint', {
                defaultValue: '模型候选会按类型筛选；每次请求只使用所选任务对应的协议。',
              })}
            </div>
          </div>

          <div className='space-y-8px' data-filtered-catalog-count={filteredCatalogSuggestions.length}>
            <div className='flex items-center justify-between gap-8px'>
              <label htmlFor={modelInputId} className='text-13px font-500 text-t-secondary'>
                {t('settings.modelSelection', { defaultValue: '模型' })}
              </label>
              {onRefreshCatalog && (
                <Tooltip content={t('common.refresh', { defaultValue: '刷新' })}>
                  <Button
                    size='mini'
                    type='text'
                    className='!h-28px !w-28px !min-w-28px'
                    icon={<Refresh theme='outline' size='14' />}
                    loading={catalogLoading}
                    onClick={onRefreshCatalog}
                    aria-label={t('common.refresh', { defaultValue: '刷新' })}
                  />
                </Tooltip>
              )}
            </div>
            <AutoComplete
              value={value.model}
              data={filteredCatalogSuggestions.map((suggestion) => ({
                value: suggestion.value,
                name: suggestion.label,
              }))}
              disabled={!primaryTask}
              loading={catalogLoading}
              allowClear
              status={primaryTask && (!value.model.trim() || duplicateModel) ? 'error' : undefined}
              placeholder={
                primaryTask
                  ? t('settings.modelSelectionPlaceholder', {
                      defaultValue: '搜索目录模型，或直接输入官网模型 ID',
                    })
                  : t('settings.modelSelectionRequiresType', {
                      defaultValue: '请先选择模型类型',
                    })
              }
              defaultActiveFirstOption={false}
              onChange={(model, option) => {
                const manualModel = resolveModelInputChange(model, option);
                if (manualModel !== undefined) onChange({ ...value, model: manualModel });
              }}
              onSelect={(model) => {
                const suggestion = filteredCatalogSuggestions.find((item) => item.value === model);
                if (suggestion) selectCatalogSuggestion(suggestion);
              }}
              triggerProps={{ getPopupContainer: () => document.body }}
              inputProps={{
                id: modelInputId,
                'aria-describedby': `${modelInputId}-hint`,
              }}
              data-unified-model-input
            />
            <div
              id={`${modelInputId}-hint`}
              role={duplicateModel ? 'alert' : 'note'}
              className={`text-11px leading-4 ${duplicateModel ? 'text-danger-6' : 'text-t-secondary'}`}
            >
              {duplicateModel
                ? t('settings.modelIdDuplicate', {
                    defaultValue: '该模型 ID 已存在。',
                  })
                : t('settings.modelSelectionHint', {
                    defaultValue: '目录仅提供当前类型的建议；没有匹配项时可直接输入模型 ID。',
                  })}
            </div>
            {primaryTask && !catalogLoading && filteredCatalogSuggestions.length === 0 && !catalogError && (
              <div className='text-11px text-t-tertiary' role='note' data-empty-filtered-model-catalog>
                {t('settings.modelCatalogFilteredEmpty', {
                  defaultValue: '目录中暂无该类型的模型，请直接输入模型 ID。',
                })}
              </div>
            )}
            {catalogError && (
              <div className='text-11px text-warning-6' role='note'>
                {t('settings.modelCatalogUnavailable', {
                  defaultValue: '目录暂不可用，不影响手填模型 ID。',
                })}{' '}
                {catalogError}
              </div>
            )}
          </div>
        </>
      ) : (
        <div className='space-y-8px'>
          <label htmlFor={modelInputId} className='text-13px font-500 text-t-secondary'>
            {t('settings.modelId', { defaultValue: '模型 ID' })}
          </label>
          <Input id={modelInputId} value={value.model} readOnly data-readonly-model-id />
        </div>
      )}

      <section className='space-y-10px' aria-labelledby={taskSectionId} data-model-task-section>
        <div className='space-y-4px'>
          <div id={taskSectionId} className='text-13px font-500 text-t-secondary'>
            {t('settings.modelSupportedTasks', { defaultValue: '支持的任务' })}
          </div>
          <div className='text-11px leading-4 text-t-secondary'>
            {t('settings.modelSupportedTasksHint', {
              defaultValue: '只添加该模型实际支持的任务；每次请求只使用当前任务对应的一套协议。',
            })}
          </div>
        </div>

        {modelReadOnly && value.capabilities.length === 0 && (
          <div className='text-12px text-t-secondary' role='note' data-empty-model-tasks>
            {t('settings.modelTasksEmpty', {
              defaultValue: '该模型尚未配置任何任务。',
            })}
          </div>
        )}

        {(modelReadOnly || value.capabilities.length > 0) && (
          <Select
            value={undefined}
            disabled={!value.model.trim() || availableTasks.length === 0}
            options={availableTasks.map((task) => ({
              value: task,
              label: t(`settings.modelTask.${task}`, { defaultValue: task }),
            }))}
            placeholder={
              availableTasks.length === 0
                ? t('settings.modelTasksAllAdded', {
                    defaultValue: '所有任务均已添加',
                  })
                : t('settings.addAnotherModelTask', {
                    defaultValue: '添加其他任务',
                  })
            }
            onChange={(task) => {
              if (typeof task === 'string') addTask(task as ModelTask);
            }}
            triggerProps={{ getPopupContainer: () => document.body }}
            aria-label={t('settings.addAnotherModelTask', {
              defaultValue: '添加其他任务',
            })}
            data-model-task-picker
          />
        )}

        {value.capabilities.map((capability) => {
        const loading = manifestLoadingTasks.includes(capability.task);
        const manifest = loading ? undefined : manifests[capability.task];
        const loadFailed = manifestErrorTasks.includes(capability.task);
        const descriptor = protocolDescriptorForDraft(capability, manifest);
        const sdkTransport = descriptor?.transport === 'sdk';
        const recommended = manifest?.recommendation?.protocol_id;
        const protocolOptions = [...(manifest?.protocols ?? [])].sort(
          (left, right) => Number(right.protocol_id === recommended) - Number(left.protocol_id === recommended)
        );
        const protocolRegistered = protocolOptions.some(
          (protocol) => protocol.protocol_id === capability.protocol
        );
        const actualBaseUrl = effectiveBaseUrl(capability, manifest, providerBaseUrl, connections);
        // Which half of the URL owns the version segment. Built-in presets ship
        // a matching default connection; a custom provider does not, so stating
        // this is the only way it learns the convention.
        const rootShape = sdkTransport ? undefined : descriptor?.root_shape ?? undefined;
        const crossOrigin = requiresCrossOriginConsent(capability, manifest, providerBaseUrl, connections);
        const providerParamsValid = parseProviderParams(capability.providerParamsJson).ok;
        const endpointDescriptors =
          descriptor?.endpoints
            .filter(
              (endpoint) =>
                endpoint.task === capability.task && isCapabilityEndpointField(endpoint.field)
            )
            .map((endpoint) => ({ ...endpoint, field: endpoint.field as CapabilityEndpointField })) ?? [];
        const availableRoles = ['default', ...connections.map((connection) => connection.role)];
        const selectedRole = capability.connectionRole || 'default';
        const selectedRoleExists = availableRoles.includes(selectedRole);
        const genericAdvancedProtocol = Boolean(
          descriptor && manifest && !descriptor.platforms.includes(manifest.platform)
        );
        const selectedAuthScheme =
          selectedRole === 'default'
            ? providerAuthScheme
            : connections.find((connection) => connection.role === selectedRole)?.auth_scheme ?? '';
        const authSchemeCompatible =
          !descriptor ||
          !selectedAuthScheme ||
          isProtocolAuthSchemeAllowed(selectedAuthScheme, descriptor.allowed_auth_schemes);
        const outputLimitRequired = descriptor?.requires_output_ceiling ?? false;
        const outputLimitMissing =
          outputLimitRequired &&
          !(typeof capability.outputLimit === 'number' && capability.outputLimit > 0);
        const recommendedConnection = descriptor?.default_connections.find(
          (connection) => (connection.connection_role ?? 'default') === selectedRole
        );
        const fallbackEndpointField: CapabilityEndpointField =
          capability.task === 'realtime_conversation' ? 'realtime_endpoint' : 'endpoint';
        const endpointFields = new Set<CapabilityEndpointField>(
          sdkTransport
            ? []
            : [
                ...endpointDescriptors.map((endpoint) => endpoint.field),
                ...storedEndpointFields(capability),
                ...(endpointDescriptors.length === 0 ? [fallbackEndpointField] : []),
              ]
        );
        const taskValidationErrors = settledValidationErrors.filter(
          (error) => error.task === capability.task && error.code !== 'manifest_loading'
        );
        const hasValidationError = taskValidationErrors.length > 0;
        const expanded = disclosureState.expandedTasks.has(capability.task);
        const preparing = loading || validationPending || recommendationPendingTasks.has(capability.task);
        const statusColor = hasValidationError
          ? 'red'
          : preparing
            ? 'arcoblue'
            : genericAdvancedProtocol
              ? 'orange'
              : 'green';
        const statusText = hasValidationError
          ? t('settings.modelAdvanced.needsAttention', {
              defaultValue: `待处理 ${taskValidationErrors.length} 项`,
              count: taskValidationErrors.length,
            })
          : preparing
            ? t('settings.modelAdvanced.preparing', { defaultValue: '正在准备默认配置' })
            : genericAdvancedProtocol
              ? t('settings.modelAdvanced.reviewCompatibility', { defaultValue: '请确认兼容性' })
              : t('settings.modelAdvanced.ready', { defaultValue: '默认配置已就绪' });
        const detailsId = `${capabilityDetailsId}-${capability.task}`;
        const protocolSummary =
          capability.protocol ||
          t('settings.modelAdvanced.protocolPending', { defaultValue: '待选择协议' });
        const baseUrlSummary = sdkTransport
          ? t('settings.modelAdvanced.sdkTransport', { defaultValue: 'SDK 连接（无需 Base URL）' })
          : compactCapabilityUrlSummary(actualBaseUrl) ||
            t('settings.modelAdvanced.baseUrlPending', { defaultValue: '待配置 Base URL' });

        return (
          <section
            key={capability.task}
            className={`overflow-hidden rounded-8px border border-solid ${
              hasValidationError ? 'border-danger-4' : 'border-[var(--color-border-2)]'
            }`}
            data-capability-card={capability.task}
            data-capability-has-error={hasValidationError}
            data-capability-expanded={expanded}
          >
            <div className='flex items-stretch' data-capability-card-header={capability.task}>
              <button
                type='button'
                className='min-w-0 flex-1 border-0 bg-transparent px-14px py-12px text-left hover:bg-fill-1 focus-visible:outline-none focus-visible:shadow-[inset_0_0_0_2px_rgba(var(--primary-6),0.48)]'
                aria-expanded={expanded}
                aria-controls={detailsId}
                data-capability-disclosure={capability.task}
                onClick={() =>
                  setDisclosureState((current) => toggleCapabilityDisclosure(current, capability.task))
                }
              >
                <div className='flex min-w-0 flex-wrap items-start justify-between gap-10px'>
                  <div className='min-w-0 flex-1'>
                    <div className='flex flex-wrap items-center gap-8px'>
                      <span className='font-600 text-t-primary'>
                        {t(`settings.modelTask.${capability.task}`, { defaultValue: capability.task })}
                      </span>
                      <Tag size='small' color={statusColor}>
                        {statusText}
                      </Tag>
                    </div>
                    <div
                      className='mt-6px flex min-w-0 flex-wrap items-center gap-x-6px gap-y-2px text-11px text-t-secondary'
                      data-capability-summary={capability.task}
                    >
                      <span className='max-w-260px truncate' title={protocolSummary}>
                        {protocolSummary}
                      </span>
                      <span aria-hidden='true'>·</span>
                      <span>{selectedRole}</span>
                      <span aria-hidden='true'>·</span>
                      <span className='max-w-300px truncate' title={baseUrlSummary}>
                        {baseUrlSummary}
                      </span>
                    </div>
                  </div>
                  <span className='ml-auto flex shrink-0 items-center gap-6px whitespace-nowrap text-12px text-primary-6'>
                    {expanded
                      ? t('settings.modelAdvanced.collapseConfiguration', { defaultValue: '收起配置' })
                      : t('settings.modelAdvanced.advancedConfiguration', { defaultValue: '高级配置' })}
                    {expanded ? <Down theme='outline' size='14' /> : <Right theme='outline' size='14' />}
                  </span>
                </div>
              </button>
              {(modelReadOnly || capability.task !== primaryTask) && (
                <div className='flex shrink-0 items-center pr-10px'>
                  <Popconfirm
                    title={t('settings.removeModelTaskConfirm', {
                      defaultValue: `移除“${t(`settings.modelTask.${capability.task}`, { defaultValue: capability.task })}”及其高级配置？`,
                      task: t(`settings.modelTask.${capability.task}`, { defaultValue: capability.task }),
                    })}
                    onOk={() => removeTask(capability.task)}
                  >
                    <Button
                      size='mini'
                      type='text'
                      status='danger'
                      className='!h-28px !w-28px !min-w-28px'
                      icon={<DeleteFour theme='outline' size='14' />}
                      aria-label={t('settings.removeModelTask', { defaultValue: '移除任务' })}
                      data-remove-model-task={capability.task}
                    />
                  </Popconfirm>
                </div>
              )}
            </div>

            <div
              id={detailsId}
              hidden={!expanded}
              className='space-y-12px border-0 border-t border-solid border-[var(--color-border-2)] p-14px'
              data-capability-details={capability.task}
            >

            <div className='space-y-6px'>
              <div className='text-12px text-t-secondary'>
                {t('settings.modelAdvanced.protocol', { defaultValue: '调用协议' })}
              </div>
              <Select
                value={capability.protocol || undefined}
                loading={loading}
                disabled={protocolOptions.length === 0}
                status={!protocolRegistered ? 'error' : undefined}
                options={protocolOptions.map((protocol) => ({
                  label: `${protocol.protocol_id} · ${
                    protocol.protocol_id === recommended
                      ? t('settings.protocolProviderRecommended', {
                          defaultValue: '当前供应商推荐',
                        })
                      : protocol.platforms.includes(manifest?.platform ?? '')
                        ? t('settings.protocolProviderVerified', {
                            defaultValue: '当前供应商已核验',
                          })
                        : t('settings.protocolGenericAdvanced', {
                            defaultValue: '通用高级',
                          })
                  }`,
                  value: protocol.protocol_id,
                }))}
                placeholder={
                  loading
                    ? t('common.loading', { defaultValue: '加载中' })
                    : t('settings.compatibleProtocolPlaceholder', { defaultValue: '选择已注册适配器' })
                }
                onChange={(protocol) => {
                  const nextCapability = changeCapabilityProtocol(
                    capability,
                    typeof protocol === 'string' ? protocol : '',
                    manifest
                  );
                  onChange({
                    ...value,
                    capabilities: value.capabilities.map((candidate) =>
                      candidate.task === capability.task ? nextCapability : candidate
                    ),
                  });
                }}
                triggerProps={{ getPopupContainer: () => document.body }}
              />
              {genericAdvancedProtocol && (
                <div className='text-11px text-warning-6' role='note' data-generic-protocol-warning>
                  {t('settings.protocolGenericCompatibilityWarning', {
                    defaultValue:
                      '该协议已注册，但尚未核验为当前供应商兼容；请自行确认协议、URL 与鉴权。',
                  })}
                </div>
              )}
              {descriptor && descriptor.allowed_auth_schemes.length > 0 && (
                <div className='text-11px text-t-tertiary' data-protocol-auth-schemes>
                  {t('settings.protocolAllowedAuthSchemes', {
                    defaultValue: '允许的鉴权格式',
                  })}
                  : {descriptor.allowed_auth_schemes.join(', ')}
                </div>
              )}
              {!authSchemeCompatible && (
                <div className='text-11px text-danger-6' role='alert' data-protocol-auth-incompatible>
                  {t('settings.protocolAuthSchemeIncompatible', {
                    defaultValue: `当前连接鉴权 ${selectedAuthScheme} 不被该协议接受。`,
                    authScheme: selectedAuthScheme,
                  })}
                </div>
              )}
              {!loading && (loadFailed || !manifest || protocolOptions.length === 0) && (
                <div className='text-11px text-danger-6' role='alert'>
                  {t('settings.noCompatibleProtocolForTask', {
                    defaultValue: '该模态仍可选择；有兼容的已注册协议后才能保存。',
                  })}
                </div>
              )}
            </div>

            {!sdkTransport && (
              <div className='space-y-6px'>
                <div className='text-12px text-t-secondary'>
                  {t('settings.modelAdvanced.baseUrl', { defaultValue: 'Base URL' })}
                </div>
                <Checkbox
                  checked={Boolean(capability.baseUrlOverride)}
                  data-base-url-override-toggle={capability.task}
                  onChange={(checked) =>
                    updateCapability(capability.task, {
                      // Promotion is explicit and user-initiated. Seeding the
                      // inherited value into `value` instead would let a single
                      // keystroke freeze a copy of the provider's Base URL that
                      // then wins at request time forever.
                      baseUrlOverride: checked ? actualBaseUrl : '',
                    })
                  }
                >
                  <span className='text-12px'>
                    {t('settings.modelAdvanced.baseUrlOverrideToggle', {
                      defaultValue: '为该模态单独指定 Base URL',
                    })}
                  </span>
                </Checkbox>
                <div className='flex items-center gap-8px'>
                  <Input
                    value={capability.baseUrlOverride}
                    placeholder={actualBaseUrl}
                    disabled={!capability.baseUrlOverride}
                    status={!actualBaseUrl ? 'error' : undefined}
                    onChange={(baseUrlOverride) => updateCapability(capability.task, { baseUrlOverride })}
                    data-effective-base-url={actualBaseUrl}
                  />
                  <Button
                    size='mini'
                    disabled={!capability.baseUrlOverride}
                    onClick={() => updateCapability(capability.task, { baseUrlOverride: '' })}
                  >
                    {t('settings.restoreProviderDefault', { defaultValue: '恢复默认' })}
                  </Button>
                </div>
                <div className='text-11px text-t-tertiary'>
                  {capability.baseUrlOverride
                    ? t('settings.modelAdvanced.baseUrlOverridden', { defaultValue: '当前为任务级覆盖值。' })
                    : t('settings.modelAdvanced.baseUrlInherited', {
                        defaultValue: '继承供应商地址；勾选上方选项才会写入任务级覆盖。',
                      })}
                </div>
                {rootShape && (
                  <div className='text-11px text-t-tertiary' data-root-shape={rootShape}>
                    {rootShape === 'versioned_root'
                      ? t('settings.modelAdvanced.rootShapeVersioned', {
                          defaultValue: '该协议要求 Base URL 自带版本段（如 …/v1），请求路径不带版本。',
                        })
                      : t('settings.modelAdvanced.rootShapeOrigin', {
                          defaultValue: '该协议的请求路径自带版本段，Base URL 请填到域名根（不要带 /v1）。',
                        })}
                  </div>
                )}
                {rootShape && actualBaseUrl.trim() && !rootMatchesShape(actualBaseUrl, rootShape) && (
                  <div className='text-11px text-warning-6' role='alert' data-root-shape-mismatch={rootShape}>
                    {rootShape === 'versioned_root'
                      ? t('settings.modelAdvanced.rootShapeMismatchVersioned', {
                          defaultValue: '当前 Base URL 没有版本段，多数供应商需要以 /v1 结尾。',
                        })
                      : t('settings.modelAdvanced.rootShapeMismatchOrigin', {
                          defaultValue: '当前 Base URL 含版本段，而该协议的路径也会带版本；重复的版本段会被自动去重。',
                        })}
                  </div>
                )}
              </div>
            )}

            <div className='space-y-6px'>
              <div className='text-12px text-t-secondary'>
                {t('settings.modelAdvanced.connectionRole', { defaultValue: '连接角色' })}
              </div>
              <Select
                value={selectedRole}
                status={!selectedRoleExists ? 'error' : undefined}
                options={[
                  ...availableRoles.map((role) => ({ label: role, value: role })),
                  ...(!selectedRoleExists ? [{ label: `${selectedRole} · 需创建`, value: selectedRole }] : []),
                ]}
                onChange={(connectionRole) =>
                  updateCapability(capability.task, {
                    connectionRole: typeof connectionRole === 'string' ? connectionRole : 'default',
                  })
                }
                triggerProps={{ getPopupContainer: () => document.body }}
              />
              {onCreateConnection && (
                <Button
                  size='mini'
                  type='outline'
                  data-create-named-connection={capability.task}
                  onClick={() =>
                    setCustomConnectionTask((current) =>
                      current === capability.task ? undefined : capability.task
                    )
                  }
                >
                  {t('settings.connections.createNamed', {
                    defaultValue: '新建命名连接',
                  })}
                </Button>
              )}
              {customConnectionTask === capability.task && onCreateConnection && (
                <InlineConnectionEditor
                  key={`${capability.task}:custom-connection`}
                  baseUrl={providerBaseUrl}
                  authScheme={providerAuthScheme || manifest?.default_auth_scheme || 'bearer'}
                  authSchemes={(manifest?.auth_schemes ?? []).map((scheme) => scheme.scheme)}
                  requiresCredentials
                  onSave={async (connection) => {
                    await onCreateConnection(connection);
                    updateCapability(capability.task, {
                      connectionRole: connection.role,
                      baseUrlOverride: '',
                      allowCrossOriginCredentials: false,
                    });
                    setCustomConnectionTask(undefined);
                  }}
                />
              )}
              {!selectedRoleExists && selectedRole !== 'default' && recommendedConnection && onCreateConnection && (
                <InlineConnectionEditor
                  key={`${capability.task}:${selectedRole}`}
                  role={selectedRole}
                  roleReadOnly
                  label={recommendedConnection.connection_label ?? undefined}
                  baseUrl={recommendedConnection.base_url}
                  authScheme={recommendedConnection.auth_scheme}
                  authSchemes={(manifest?.auth_schemes ?? []).map((scheme) => scheme.scheme)}
                  requiresCredentials={recommendedConnection.requires_credentials}
                  onSave={onCreateConnection}
                />
              )}
              {!selectedRoleExists && selectedRole !== 'default' && (!recommendedConnection || !onCreateConnection) && (
                <div className='text-11px text-danger-6' role='alert'>
                  {t('settings.connections.missingRole', {
                    defaultValue: '该协议需要尚未配置的连接角色；创建连接后才能保存模型。',
                  })}
                </div>
              )}
            </div>

            {[...endpointFields].map((field) => {
              const endpointDescriptor: CapabilityEndpointDescriptor =
                endpointDescriptors.find((endpoint) => endpoint.field === field) ?? {
                  task: capability.task,
                  field,
                  purpose: 'submit' as const,
                  method: null,
                  default_value: '',
                  // No manifest entry means no declared convention; assume the
                  // root carries the version, which is the OpenAI-compatible
                  // majority and matches an empty template.
                  root_shape: 'versioned_root' as const,
                  allowed_placeholders: [],
                  required_placeholders: [],
                  editable: true,
                };
              const key = draftKeyForEndpoint(field);
              const effectiveValue = endpointDescriptorValue(capability, endpointDescriptor);
              const overrideValue = capability[key];
              const resolvedUrl = resolvedCapabilityUrl(
                capability,
                endpointDescriptor,
                manifest,
                providerBaseUrl,
                connections
              );
              return (
                <div key={field} className='space-y-6px'>
                  <div className='flex items-center gap-6px text-12px text-t-secondary'>
                    <span>{endpointLabel(endpointDescriptor, capability.task)}</span>
                    {endpointDescriptor.method && <Tag size='small'>{endpointDescriptor.method}</Tag>}
                  </div>
                  <div className='flex items-center gap-8px'>
                    <Input
                      // The protocol default is a PLACEHOLDER, never a value.
                      // Rendering it as the value made it look like the user's
                      // own setting, inviting a "correction" to the provider's
                      // documented `/v1/...` path — the edit that used to
                      // manufacture a doubled version segment.
                      value={overrideValue}
                      placeholder={effectiveValue}
                      readOnly={!endpointDescriptor.editable}
                      onChange={(next) => updateCapability(capability.task, { [key]: next })}
                      data-endpoint-field={field}
                      data-endpoint-override={Boolean(overrideValue)}
                    />
                    {endpointDescriptor.editable && (
                      <Button
                        size='mini'
                        disabled={!overrideValue}
                        onClick={() => updateCapability(capability.task, { [key]: '' })}
                      >
                        {t('settings.restoreProtocolDefault', { defaultValue: '恢复推荐' })}
                      </Button>
                    )}
                  </div>
                  {resolvedUrl && (
                    <div
                      className='text-11px text-t-tertiary break-all'
                      data-resolved-endpoint-url={field}
                    >
                      {t('settings.modelAdvanced.resolvedUrl', { defaultValue: '实际请求地址' })}:{' '}
                      <span className='text-t-secondary'>{resolvedUrl}</span>
                    </div>
                  )}
                </div>
              );
            })}

            <div className='space-y-6px'>
              <div className='text-12px text-t-secondary'>
                {t('settings.modelTraitsLabel', { defaultValue: '能力细化（traits）' })}
              </div>
              <Select
                mode='multiple'
                value={capability.traits}
                options={MODEL_TRAIT_ORDER.map((trait) => ({
                  value: trait,
                  label: t(`settings.modelTrait.${trait}`, { defaultValue: trait }),
                }))}
                onChange={(traits: ModelTrait[]) =>
                  updateCapability(capability.task, {
                    traits: MODEL_TRAIT_ORDER.filter((trait) => (traits ?? []).includes(trait)),
                  })
                }
                triggerProps={{ getPopupContainer: () => document.body }}
              />
            </div>

            <div className='space-y-6px'>
              <div className='text-12px text-t-secondary'>
                {t('settings.contextLimit', { defaultValue: '上下文窗口（tokens）' })}
              </div>
              <ContextLimitSelect
                value={capability.contextLimit}
                onChange={(contextLimit) => updateCapability(capability.task, { contextLimit })}
              />
            </div>

            <div className='space-y-6px'>
              <div className='text-12px text-t-secondary'>
                {t('settings.outputLimit', { defaultValue: 'Max output tokens' })}
              </div>
              <OutputLimitInput
                value={capability.outputLimit}
                onChange={(outputLimit) => updateCapability(capability.task, { outputLimit })}
              />
              {outputLimitMissing && (
                <div className='text-11px text-danger-6' role='alert' data-output-limit-required>
                  {t('settings.outputLimitRequired', {
                    defaultValue: 'This protocol requires an explicit max output token value.',
                  })}
                </div>
              )}
            </div>

            {crossOrigin && (
              <div className='rounded-8px bg-warning-1 p-10px space-y-6px'>
                <Checkbox
                  checked={capability.allowCrossOriginCredentials}
                  onChange={(allowCrossOriginCredentials) =>
                    updateCapability(capability.task, { allowCrossOriginCredentials })
                  }
                >
                  {t('settings.modelAdvanced.allowCrossOriginCredentials', {
                    defaultValue: '我确认允许向该跨域地址发送供应商凭据',
                  })}
                </Checkbox>
                {!capability.allowCrossOriginCredentials && (
                  <div className='text-11px text-danger-6' role='alert'>
                    {t('settings.modelAdvanced.crossOriginConsentRequired', {
                      defaultValue: '覆盖地址与供应商域名不同，必须明确确认后才能保存。',
                    })}
                  </div>
                )}
              </div>
            )}

            {capability.task === 'speech_synthesis' &&
              ttsSupportsProviderParamVoice(capability.protocol) && (
              <div className='space-y-6px'>
                <div className='text-12px text-t-secondary'>
                  {t('settings.modelAdvanced.defaultVoice', { defaultValue: '默认音色' })}
                </div>
                <Select
                  showSearch
                  allowCreate
                  allowClear
                  // `''`, never `undefined`: Arco's useMergeValue falls back to
                  // its own internal state for an undefined value, which would
                  // display a voice that was never written to the JSON.
                  value={providerParamVoice(capability.providerParamsJson)}
                  // While the raw JSON is unparseable the writer cannot merge a
                  // voice into it without discarding what the user typed, so
                  // the control would silently no-op. Say so instead.
                  disabled={!providerParamsValid}
                  placeholder={t('settings.modelAdvanced.defaultVoicePlaceholder', {
                    defaultValue: '选择或输入供应商音色 id',
                  })}
                  options={ttsVoiceOptionsFor(capability.protocol, value.model).map((voice) => ({
                    value: voice,
                    label: voice,
                  }))}
                  onChange={(voice?: string) =>
                    updateCapability(capability.task, {
                      providerParamsJson: withProviderParamVoice(
                        capability.providerParamsJson,
                        voice ?? ''
                      ),
                    })
                  }
                  triggerProps={{ getPopupContainer: () => document.body }}
                />
                <div className='text-11px text-t-tertiary'>
                  {providerParamsValid
                    ? t('settings.modelAdvanced.defaultVoiceHint', {
                        defaultValue:
                          '部分供应商（如 StepFun）必须提供音色，否则语音合成会直接失败。留空则由每次请求自行指定。',
                      })
                    : t('settings.modelAdvanced.defaultVoiceUnavailable', {
                        defaultValue: '下方的供应商参数 JSON 无效，修正后才能选择音色。',
                      })}
                </div>
              </div>
            )}

            {capability.protocol === 'openai.responses' && (
              <div
                className='rounded-8px bg-fill-1 p-10px space-y-6px'
                data-chain-rounds-control
                data-chain-rounds-json-valid={providerParamsValid ? 'true' : 'false'}
                data-chain-rounds-enabled={providerParamChainRounds(capability.providerParamsJson) ? 'true' : 'false'}
              >
                <Checkbox
                  checked={providerParamChainRounds(capability.providerParamsJson)}
                  disabled={!providerParamsValid}
                  onChange={(enabled) =>
                    updateCapability(capability.task, {
                      providerParamsJson: withProviderParamChainRounds(
                        capability.providerParamsJson,
                        enabled
                      ),
                    })
                  }
                >
                  {t('settings.modelAdvanced.chainRounds', {
                    defaultValue: 'Chain turns with previous_response_id (sets store: true)',
                  })}
                </Checkbox>
                <div className={`text-11px ${providerParamsValid ? 'text-t-tertiary' : 'text-danger-6'}`}>
                  {providerParamsValid
                    ? t('settings.modelAdvanced.chainRoundsRetention', {
                        defaultValue:
                          'Opt-in: provider-retained response data may be kept for at least 30 days. previous_response_id links rounds but does not reduce billed input tokens.',
                      })
                    : t('settings.modelAdvanced.chainRoundsUnavailable', {
                        defaultValue: 'Fix the provider parameters JSON below before changing this option.',
                      })}
                </div>
              </div>
            )}

            <div className='space-y-6px' data-provider-params-json>
              <div className='text-12px text-t-secondary'>
                {t('settings.modelAdvanced.params', { defaultValue: '供应商参数 JSON' })}
              </div>
              <Input.TextArea
                value={capability.providerParamsJson}
                rows={4}
                status={providerParamsValid ? undefined : 'error'}
                placeholder='{\n  "voice": "alloy"\n}'
                onChange={(providerParamsJson) => updateCapability(capability.task, { providerParamsJson })}
              />
              <div className={`text-11px ${providerParamsValid ? 'text-t-tertiary' : 'text-danger-6'}`}>
                {providerParamsValid
                  ? t('settings.modelAdvanced.providerParamsOnly', {
                      defaultValue: '只填写供应商原始参数；协议、URL 和 endpoint 由上方结构化字段管理。',
                    })
                  : t('settings.modelAdvanced.invalidParamsJson', {
                      defaultValue: '必须是合法的 JSON 对象。',
                    })}
              </div>
            </div>
            </div>
          </section>
        );
      })}
      </section>
    </div>
  );
};

export default ModelDefinitionEditor;
