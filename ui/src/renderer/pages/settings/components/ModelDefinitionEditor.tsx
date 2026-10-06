/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { MODEL_TASK_ORDER } from '@/common/modelCapabilities';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ModelTaskSource } from '@/common/protocolBindings/ModelTaskSource';
import type { ModelCatalogSource } from '@/common/protocolBindings/ModelCatalogSource';
import type { ModelTrait } from '@/common/protocolBindings/ModelTrait';
import { ttsSupportsProviderParamVoice, ttsVoiceOptionsFor } from '@/renderer/components/model/ttsVoiceOptions';
import { AutoComplete, Button, Checkbox, Input, Popconfirm, Select, Tag, Tooltip } from '@arco-design/web-react';
import { CheckOne, Code, DeleteFour, Down, Left, LinkOne, Refresh, Right, Search, Shield, TagOne } from '@icon-park/react';
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
  acknowledgeCatalogTaskConflict,
  applyCatalogSuggestion,
  capabilityValidationMessageKey,
  changeCapabilityProtocol,
  changeModelDefinitionId,
  effectiveBaseUrl,
  endpointDescriptorValue,
  isCapabilityEndpointField,
  isDuplicateModelId,
  getCatalogTaskConflict,
  isProtocolAuthSchemeAllowed,
  parseProviderParams,
  patchCapabilityDraft,
  providerParamChainRounds,
  providerParamReasoningEffort,
  protocolDescriptorForDraft,
  reasoningEffortsForProtocol,
  protocolSupportsReasoningEffort,
  providerParamVoice,
  reconcileCapabilityRecommendations,
  removeCapabilityTask,
  resolveModelInputChange,
  requiresCrossOriginConsent,
  resolvedCapabilityUrl,
  isValidModelTokenLimit,
  rootMatchesShape,
  withProviderParamVoice,
  withProviderParamChainRounds,
  withProviderParamReasoningEffort,
  withCatalogTaskEvidence,
  type CapabilityEndpointDescriptor,
  type CapabilityEndpointField,
  type CapabilityValidationError,
  type CapabilityValidationResult,
  type ModelCapabilityDraft,
  type ModelCapabilityDraftPatch,
  type ModelDefinitionDraft,
  type ModelReasoningEffort,
  type ModelProtocolManifestMap,
  type ProviderConnectionDescriptor,
  type ProviderConnectionInput,
} from './providerModelAdvanced';

export interface ModelCatalogSuggestion {
  value: string;
  label: string;
  displayName?: string;
  tasks: ModelTask[];
  traits: ModelTrait[];
  tasksSource?: ModelTaskSource;
  /** Window the provider's own catalog declares, when it declares one. */
  contextLimit?: number;
  outputLimit?: number;
  contextLimitKind?: 'input_only' | 'combined';
}

export interface ModelDefinitionEditorProps {
  value: ModelDefinitionDraft;
  onChange: React.Dispatch<React.SetStateAction<ModelDefinitionDraft>>;
  providerBaseUrl: string;
  providerAuthScheme: string;
  providerLabel?: string;
  manifests: ModelProtocolManifestMap;
  manifestLoadingTasks?: readonly ModelTask[];
  manifestErrorTasks?: readonly ModelTask[];
  validationErrors: CapabilityValidationResult['errors'];
  validationPending?: boolean;
  existingModelIds?: readonly string[];
  modelReadOnly?: boolean;
  /** Scenario editors keep the requested task fixed while retaining the full draft. */
  capabilityTask?: ModelTask;
  catalogSuggestions?: readonly ModelCatalogSuggestion[];
  catalogLoading?: boolean;
  catalogError?: string;
  catalogSource?: ModelCatalogSource;
  catalogFetchReady?: boolean;
  onRefreshCatalog?: () => Promise<unknown> | void;
  connections?: readonly ProviderConnectionDescriptor[];
  onCreateConnection?: (connection: ProviderConnectionInput) => Promise<void>;
  onCallConfigFocusChange?: (task?: ModelTask) => void;
  callConfigFooterPlacement?: 'internal' | 'modal';
}

export interface ModelDefinitionEditorHandle {
  cancelCallConfig: () => void;
  applyCallConfig: () => void;
}

type CallConfigIntent = 'overview' | 'connection' | 'limits' | 'protocol' | 'diagnostics';

const callConfigIntentForError = (code: CapabilityValidationError): CallConfigIntent => {
  switch (code) {
    case 'connection_role_required':
    case 'connection_missing':
      return 'connection';
    case 'output_ceiling_required':
    case 'invalid_token_limit':
      return 'limits';
    case 'protocol_required':
    case 'protocol_not_registered':
    case 'protocol_task_mismatch':
    case 'auth_scheme_incompatible':
    case 'base_url_required':
    case 'cross_origin_consent_required':
    case 'invalid_provider_params':
      return 'protocol';
    default:
      return 'diagnostics';
  }
};

const cloneCapabilityDraft = (capability: ModelCapabilityDraft): ModelCapabilityDraft => ({
  ...capability,
  traits: [...capability.traits],
});

const sameCapabilityDraft = (
  left: ModelCapabilityDraft | undefined,
  right: ModelCapabilityDraft
): boolean => left !== undefined && JSON.stringify(left) === JSON.stringify(right);

const compactTokenCount = (value: number | undefined, fallback: string): string => {
  if (value === undefined) return fallback;
  if (value >= 1_000_000 && value % 1_000_000 === 0) return `${value / 1_000_000}M`;
  if (value >= 1_000 && value % 1_000 === 0) return `${value / 1_000}k`;
  return new Intl.NumberFormat().format(value);
};

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
      candidate.transportSource === capability.transportSource &&
      candidate.protocol === capability.protocol &&
      candidate.connectionRole === capability.connectionRole &&
      candidate.baseUrlOverride === capability.baseUrlOverride
    );
  });

const EMPTY_TASKS: readonly ModelTask[] = [];
const EMPTY_IDS: readonly string[] = [];
const EMPTY_CATALOG: readonly ModelCatalogSuggestion[] = [];
const EMPTY_CONNECTIONS: readonly ProviderConnectionDescriptor[] = [];

/**
 * Shared provider-model editor used by create-provider, add-model, and edit-model.
 * Catalog data is advisory. Model IDs are always editable; optional invocation
 * routes do not filter the catalog or gate the model's native Chat abilities.
 */
const ModelDefinitionEditor = React.forwardRef<ModelDefinitionEditorHandle, ModelDefinitionEditorProps>(({
  value,
  onChange,
  providerBaseUrl,
  providerAuthScheme,
  providerLabel,
  manifests,
  manifestLoadingTasks = EMPTY_TASKS,
  manifestErrorTasks = EMPTY_TASKS,
  validationErrors,
  validationPending = false,
  existingModelIds = EMPTY_IDS,
  modelReadOnly = false,
  capabilityTask,
  catalogSuggestions = EMPTY_CATALOG,
  catalogLoading = false,
  catalogError,
  catalogSource,
  catalogFetchReady = true,
  onRefreshCatalog,
  connections = EMPTY_CONNECTIONS,
  onCreateConnection,
  onCallConfigFocusChange,
  callConfigFooterPlacement = 'internal',
}, ref) => {
  const { t } = useTranslation();
  const modelInputId = useId();
  const modelAliasInputId = `${modelInputId}-alias`;
  const modelAliasPanelId = `${modelInputId}-alias-panel`;
  const [catalogPopupVisible, setCatalogPopupVisible] = useState(false);
  const [browseAllCatalog, setBrowseAllCatalog] = useState(false);
  const [catalogRefreshPending, setCatalogRefreshPending] = useState(false);
  const [catalogWasRefreshed, setCatalogWasRefreshed] = useState(false);
  const [localCatalogError, setLocalCatalogError] = useState('');
  const catalogBusy = catalogLoading || catalogRefreshPending;
  const effectiveCatalogError = catalogError || localCatalogError;
  const catalogQuery = browseAllCatalog ? '' : value.model.trim().toLowerCase();
  const matchingCatalogCount = catalogSuggestions.filter((suggestion) =>
    suggestion.value.toLowerCase().includes(catalogQuery) || suggestion.label.toLowerCase().includes(catalogQuery)
  ).length;
  const [addingCallRoute, setAddingCallRoute] = useState(false);
  const capabilityDetailsId = useId();
  const modelAlias = value.displayName?.trim() ?? '';
  const taskConflict = getCatalogTaskConflict(value);
  const selectedCatalogEntry = catalogSuggestions.find((entry) => entry.value.trim() === value.model.trim());
  useEffect(() => {
    if (selectedCatalogEntry) {
      onChange((current) => withCatalogTaskEvidence(current, { ...selectedCatalogEntry, model: selectedCatalogEntry.value }));
    }
  }, [onChange, selectedCatalogEntry]);
  const [modelAliasExpanded, setModelAliasExpanded] = useState(false);
  const modelAliasActionLabel = modelAlias
    ? `${t('settings.editModelDisplayName', { defaultValue: '编辑模型别名' })}：${modelAlias}`
    : t('settings.addModelDisplayName', { defaultValue: '添加模型别名' });
  const [customConnectionTask, setCustomConnectionTask] = useState<ModelTask>();
  const selectedTasks = useMemo(
    () => value.capabilities.filter((capability) => capabilityTask === undefined || capability.task === capabilityTask).map((capability) => capability.task),
    [value.capabilities, capabilityTask]
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
  // Chat token settings are editable on the model homepage. Their errors
  // still block saving, but should not open a separate invocation page.
  const disclosureValidationErrors = useMemo(
    () => settledValidationErrors.filter((error) =>
      (capabilityTask === undefined || !error.task || error.task === capabilityTask) &&
      (error.task !== 'chat' || !['output_ceiling_required', 'invalid_token_limit'].includes(error.code))
    ),
    [settledValidationErrors, capabilityTask]
  );
  const [disclosureState, setDisclosureState] = useState(() =>
    createCapabilityDisclosureState(selectedTasks, disclosureValidationErrors)
  );
  const [callConfigIntentByTask, setCallConfigIntentByTask] = useState<
    Partial<Record<ModelTask, CallConfigIntent>>
  >({});
  const [callConfigBaselineByTask, setCallConfigBaselineByTask] = useState<
    Partial<Record<ModelTask, ModelCapabilityDraft>>
  >({});
  const [focusedCallConfigTask, setFocusedCallConfigTask] = useState<ModelTask>();
  const [editingOutputLimitByTask, setEditingOutputLimitByTask] = useState<
    Partial<Record<ModelTask, boolean>>
  >({});
  const [protocolTransportOpenByTask, setProtocolTransportOpenByTask] = useState<
    Partial<Record<ModelTask, boolean>>
  >({});
  const [protocolParamsOpenByTask, setProtocolParamsOpenByTask] = useState<
    Partial<Record<ModelTask, boolean>>
  >({});

  useEffect(() => {
    if (recommendationPending) {
      // Reconcile against the latest parent state. A manifest response can land
      // in the same frame as model typing or an advanced-field edit; replaying
      // the render-time `value` snapshot would otherwise overwrite that input.
      onChange((current) => {
        const capabilities = reconcileCapabilityRecommendations(
          current.capabilities,
          recommendationManifests
        );
        return sameCapabilities(current.capabilities, capabilities)
          ? current
          : { ...current, capabilities };
      });
    }
  }, [onChange, recommendationManifests, recommendationPending]);

  useEffect(() => {
    setDisclosureState((current) =>
      syncCapabilityDisclosureState(current, selectedTasks, disclosureValidationErrors)
    );
  }, [selectedTasks, disclosureValidationErrors]);

  useEffect(() => {
    const nextIntents: Partial<Record<ModelTask, CallConfigIntent>> = {};
    for (const error of disclosureValidationErrors) {
      if (error.task && nextIntents[error.task] === undefined) {
        nextIntents[error.task] = callConfigIntentForError(error.code);
      }
    }
    if (Object.keys(nextIntents).length > 0) {
      setCallConfigIntentByTask((current) => ({ ...current, ...nextIntents }));
    }
  }, [disclosureValidationErrors]);

  const duplicateModel = isDuplicateModelId(value.model, existingModelIds);
  // Show the missing-ID error after a purpose is configured. The initial
  // empty form keeps the ID input available without an error.
  const missingModel = value.capabilities.length > 0 && !value.model.trim();

  const updateCapability = (task: ModelTask, patch: ModelCapabilityDraftPatch) => {
    onChange((current) => ({
      ...current,
      capabilities: current.capabilities.map((capability) =>
        capability.task === task ? patchCapabilityDraft(capability, patch) : capability
      ),
    }));
  };

  const toggleCallConfig = (task: ModelTask) => {
    const expanded = disclosureState.expandedTasks.has(task);
    if (expanded && focusedCallConfigTask === task) {
      finishCallConfig(task, false);
      return;
    }
    if (!expanded) {
      const capability = value.capabilities.find((candidate) => candidate.task === task);
      if (capability) {
        setCallConfigBaselineByTask((current) =>
          current[task] ? current : { ...current, [task]: cloneCapabilityDraft(capability) }
        );
      }
      setCallConfigIntentByTask((current) => ({
        ...current,
        [task]: current[task] ?? 'overview',
      }));
      setFocusedCallConfigTask(task);
      onCallConfigFocusChange?.(task);
    }
    setDisclosureState((current) => toggleCapabilityDisclosure(current, task));
  };

  const finishCallConfig = (task: ModelTask, restoreBaseline: boolean) => {
    const baseline = callConfigBaselineByTask[task];
    if (restoreBaseline && baseline) {
      onChange((current) => ({
        ...current,
        capabilities: current.capabilities.map((capability) =>
          capability.task === task ? cloneCapabilityDraft(baseline) : capability
        ),
      }));
    }
    setCallConfigBaselineByTask((current) => {
      const next = { ...current };
      delete next[task];
      return next;
    });
    setDisclosureState((current) =>
      current.expandedTasks.has(task) ? toggleCapabilityDisclosure(current, task) : current
    );
    setFocusedCallConfigTask(undefined);
    onCallConfigFocusChange?.(undefined);
  };

  React.useImperativeHandle(
    ref,
    () => ({
      cancelCallConfig: () => {
        if (focusedCallConfigTask) finishCallConfig(focusedCallConfigTask, true);
      },
      applyCallConfig: () => {
        if (focusedCallConfigTask) finishCallConfig(focusedCallConfigTask, false);
      },
    }),
    [focusedCallConfigTask, callConfigBaselineByTask]
  );

  const selectCatalogSuggestion = (profile: ModelCatalogSuggestion) => {
    onChange((current) => applyCatalogSuggestion(current, {
      model: profile.value,
      ...(profile.displayName ? { displayName: profile.displayName } : {}),
      tasks: profile.tasks,
      traits: profile.traits,
      tasksSource: profile.tasksSource,
      ...(profile.contextLimit === undefined ? {} : { contextLimit: profile.contextLimit }),
      ...(profile.outputLimit === undefined ? {} : { outputLimit: profile.outputLimit }),
      ...(profile.contextLimitKind === undefined ? {} : { contextLimitKind: profile.contextLimitKind }),
    }));
  };
  const removeTask = (task: ModelTask) => {
    onChange((current) => ({
      ...current,
      capabilities: removeCapabilityTask(current.capabilities, task),
    }));
  };

  const addCallRoute = (task: ModelTask) => {
    onChange((current) => {
      const next = { ...current, capabilities: addCapabilityTask(current.capabilities, task) };
      const entry = catalogSuggestions.find((profile) => profile.value.trim() === current.model.trim());
      return entry ? applyCatalogSuggestion(next, {
        model: current.model,
        displayName: current.displayName,
        tasks: entry.tasks,
        traits: entry.traits,
        tasksSource: entry.tasksSource,
        contextLimit: entry.contextLimit,
        outputLimit: entry.outputLimit,
        contextLimitKind: entry.contextLimitKind,
      }) : next;
    });
    setAddingCallRoute(false);
  };

  const refreshCatalog = async () => {
    if (!onRefreshCatalog || !catalogFetchReady || catalogBusy) return;
    setBrowseAllCatalog(true);
    setCatalogPopupVisible(true);
    setCatalogWasRefreshed(true);
    setCatalogRefreshPending(true);
    setLocalCatalogError('');
    try {
      await onRefreshCatalog();
    } catch {
      // SWR reports provider errors through catalogError. Other callers can
      // reject without an error prop; keep that failure visible as well.
      setLocalCatalogError(t('settings.modelCatalogFetchFailed'));
    } finally {
      setCatalogRefreshPending(false);
    }
  };

  const catalogStatus = catalogBusy
    ? t('settings.modelCatalogLoading', { defaultValue: '正在获取供应商模型列表…' })
    : !catalogFetchReady
      ? t('settings.modelCatalogNeedsConfiguration', { defaultValue: '填写 API 地址和凭据后可获取模型列表，也可直接输入模型 ID。' })
      : effectiveCatalogError
        ? effectiveCatalogError
        : catalogSuggestions.length === 0
          ? t('settings.modelCatalogEmpty', { defaultValue: '暂无模型列表，请直接输入模型 ID，或刷新重试。' })
          : catalogSource === 'official_documentation'
            ? t('settings.modelCatalogReference', { count: catalogSuggestions.length, defaultValue: '官方文档建议（{{count}} 个模型），并非账号实时列表；也可直接输入其他模型 ID。' })
            : t('settings.modelCatalogLoaded', { count: catalogSuggestions.length, defaultValue: '已获取 {{count}} 个模型，可从列表选择或继续手填。' });

  return (
    <div className='flex flex-col gap-16px' data-model-definition-editor>
      {!modelReadOnly ? (
        <div
          hidden={focusedCallConfigTask !== undefined}
          className='space-y-8px'
          data-model-catalog-count={catalogSuggestions.length}
        >
          <div className='flex items-center justify-between gap-8px'>
            <div className='flex min-w-0 items-center gap-8px'>
              <label htmlFor={modelInputId} className='text-13px font-500 text-t-secondary'>
                {t('settings.modelSelection', { defaultValue: '模型 ID' })}
              </label>
              {catalogSource === 'official_documentation' && catalogSuggestions.length > 0 && (
                <Tooltip content={t('settings.modelCatalogReference', { count: catalogSuggestions.length })}>
                  <span
                    className='rounded-4px bg-fill-2 px-6px py-2px text-11px text-t-secondary'
                    tabIndex={0}
                    aria-label={t('settings.modelCatalogReference', { count: catalogSuggestions.length })}
                    data-model-catalog-reference
                  >
                    {t('settings.modelCatalogReferenceCompact', { count: catalogSuggestions.length, defaultValue: '官方建议 · {{count}}' })}
                  </span>
                </Tooltip>
              )}
            </div>
            {onRefreshCatalog && (
              <Tooltip content={catalogFetchReady ? t('settings.refreshModelCatalog', { defaultValue: '获取模型列表' }) : catalogStatus}>
                <Button
                  size='mini'
                  type='text'
                  className='!h-28px !w-28px !min-w-28px'
                  icon={<Refresh theme='outline' size='14' />}
                  loading={catalogBusy}
                  disabled={!catalogFetchReady}
                  onClick={() => void refreshCatalog()}
                  aria-label={t('settings.refreshModelCatalog', { defaultValue: '获取模型列表' })}
                  data-refresh-model-catalog
                />
              </Tooltip>
            )}
          </div>
          <div className='flex min-w-0 items-center gap-6px' data-model-id-controls>
            <AutoComplete
              className='min-w-0 flex-1'
              value={value.model}
              data={catalogSuggestions.map((suggestion) => ({
                value: suggestion.value,
                name: suggestion.label,
              }))}
              loading={catalogBusy}
              filterOption={browseAllCatalog ? false : (input, option) => {
                const query = input.toLowerCase();
                const optionValue = String((option.props as { value?: unknown }).value ?? '');
                const label = catalogSuggestions.find((suggestion) => suggestion.value === optionValue)?.label ?? optionValue;
                return optionValue.toLowerCase().includes(query) || label.toLowerCase().includes(query);
              }}
              onFocus={() => setCatalogPopupVisible(true)}
              onSearch={() => {
                setBrowseAllCatalog(false);
                setCatalogPopupVisible(true);
              }}
              dropdownRender={(menu) => (
                <div data-model-catalog-dropdown>
                  <div className={`px-12px py-8px text-12px ${effectiveCatalogError ? 'text-warning-6' : 'text-t-secondary'}`} role={effectiveCatalogError ? 'alert' : 'status'}>
                    {catalogStatus}
                  </div>
                  {menu}
                  {!catalogBusy && !effectiveCatalogError && catalogSuggestions.length > 0 && matchingCatalogCount === 0 && (
                    <div className='px-12px py-8px text-12px text-t-secondary'>
                      {t('settings.modelCatalogNoMatch', { defaultValue: '列表中没有匹配项，可以直接使用您输入的模型 ID。' })}
                    </div>
                  )}
                </div>
              )}
              allowClear
              status={
                value.capabilities.length > 0 && (!value.model.trim() || duplicateModel)
                  ? 'error'
                  : undefined
              }
              placeholder={t('settings.modelSelectionPlaceholder', {
                defaultValue: '搜索供应商模型，或直接输入模型 ID',
              })}
              defaultActiveFirstOption={false}
              onChange={(model, option) => {
                const manualModel = resolveModelInputChange(model, option);
                if (manualModel !== undefined) {
                  onChange((current) =>
                    current.model === manualModel
                      ? current
                      : withCatalogTaskEvidence(
                          changeModelDefinitionId(current, manualModel),
                          (() => {
                            const entry = catalogSuggestions.find((candidate) => candidate.value.trim() === manualModel.trim());
                            return entry ? { ...entry, model: entry.value } : undefined;
                          })()
                        )
                  );
                }
              }}
              onSelect={(model) => {
                const suggestion = catalogSuggestions.find((item) => item.value === model);
                if (suggestion) selectCatalogSuggestion(suggestion);
                setCatalogPopupVisible(false);
              }}
              triggerProps={{
                getPopupContainer: () => document.body,
                popupVisible: catalogPopupVisible,
                onVisibleChange: setCatalogPopupVisible,
              }}
              inputProps={{
                id: modelInputId,
                'aria-describedby': `${modelInputId}-hint`,
                onKeyDown: (event) => {
                  if (event.key === 'Escape') setCatalogPopupVisible(false);
                },
                suffix: (
                  <Button
                    type='text'
                    size='mini'
                    icon={<Down theme='outline' size='14' />}
                    aria-label={t('settings.browseModelCatalog', { defaultValue: '查看模型列表' })}
                    aria-expanded={catalogPopupVisible}
                    data-browse-model-catalog
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => {
                      setBrowseAllCatalog(true);
                      setCatalogPopupVisible((visible) => !visible);
                      if (catalogSuggestions.length === 0 && !catalogBusy) void refreshCatalog();
                    }}
                  />
                ),
              }}
              data-unified-model-input
            />
            <Tooltip
              mini
              position='top'
              content={
                <span className='block max-w-240px'>
                  <span className='block font-500'>{modelAliasActionLabel}</span>
                  <span className='mt-2px block text-11px opacity-80'>
                    {t('settings.modelDisplayNameHint', {
                      defaultValue: '非必填，仅用于界面展示；实际请求仍使用原始模型 ID。',
                    })}
                  </span>
                </span>
              }
            >
              <Button
                size='small'
                type='secondary'
                className={`!h-32px !w-32px !min-w-32px !shrink-0 !p-0 ${
                  modelAliasExpanded || modelAlias
                    ? '!bg-primary-1 !text-primary-6'
                    : '!bg-fill-1 !text-t-tertiary hover:!bg-fill-2 hover:!text-t-secondary'
                }`}
                icon={<TagOne theme='outline' size='15' />}
                aria-label={modelAliasActionLabel}
                aria-expanded={modelAliasExpanded}
                aria-controls={modelAliasPanelId}
                data-model-alias-disclosure
                data-model-alias-configured={Boolean(modelAlias)}
                onClick={() => setModelAliasExpanded((expanded) => !expanded)}
              />
            </Tooltip>
          </div>
          <div
            id={modelAliasPanelId}
            hidden={!modelAliasExpanded}
            className='space-y-4px'
            data-model-alias-panel
          >
            {modelAliasExpanded && (
              <>
                <Input
                  id={modelAliasInputId}
                  value={value.displayName ?? ''}
                  placeholder={t('settings.modelDisplayNamePlaceholder', {
                    defaultValue: '例如：Seedance 1.5 Pro',
                  })}
                  maxLength={128}
                  allowClear
                  autoFocus
                  onChange={(displayName) =>
                    onChange((current) => ({
                      ...current,
                      displayName,
                    }))
                  }
                  onKeyDown={(event) => {
                    if (event.key === 'Escape') {
                      event.preventDefault();
                      setModelAliasExpanded(false);
                    }
                  }}
                  aria-label={t('settings.modelDisplayNameTitle', {
                    defaultValue: '模型别名（选填）',
                  })}
                  aria-describedby={`${modelAliasInputId}-hint`}
                  data-model-alias-input
                />
                <div
                  id={`${modelAliasInputId}-hint`}
                  className='text-11px leading-4 text-t-tertiary'
                  role='note'
                >
                  {t('settings.modelDisplayNameHint', {
                    defaultValue: '非必填，仅用于界面展示；实际请求仍使用原始模型 ID。',
                  })}
                </div>
              </>
            )}
          </div>
          <div
            id={`${modelInputId}-hint`}
            role={duplicateModel || missingModel ? 'alert' : 'note'}
            className={`text-11px leading-4 ${
              duplicateModel || missingModel ? 'text-danger-6' : 'text-t-secondary'
            }`}
          >
            {duplicateModel
              ? t('settings.modelIdDuplicate', {
                  defaultValue: '该模型 ID 已存在。',
                })
              : missingModel
                ? // The field already turns red here, but a red border alone left
                  // "save does nothing" unexplained: the modal only answers with a
                  // generic "finish configuring each task" toast.
                  t('settings.modelIdRequired', {
                    defaultValue: '请填写模型 ID，否则无法保存。',
                  })
                : t('settings.modelSelectionHint', {
                    defaultValue: '可从列表选择，也可直接输入模型 ID。',
                  })}
          </div>
          {(catalogBusy || effectiveCatalogError || (catalogWasRefreshed && catalogSource !== 'official_documentation') || !catalogFetchReady || catalogSuggestions.length === 0) && (
            <div className={`text-11px leading-4 ${effectiveCatalogError ? 'text-warning-6' : 'text-t-tertiary'}`} role={effectiveCatalogError ? 'alert' : 'status'} data-model-catalog-status>
              {catalogStatus}
            </div>
          )}
        </div>
      ) : (
        <div hidden={focusedCallConfigTask !== undefined} className='space-y-8px'>
          <label htmlFor={modelInputId} className='text-13px font-500 text-t-secondary'>
            {t('settings.modelId', { defaultValue: '模型 ID' })}
          </label>
          <Input id={modelInputId} value={value.model} readOnly data-readonly-model-id />
        </div>
      )}

      {modelReadOnly && (
        <div hidden={focusedCallConfigTask !== undefined} className='space-y-8px'>
          <label htmlFor={modelAliasInputId} className='text-13px font-500 text-t-secondary'>
            {t('settings.modelDisplayNameTitle', { defaultValue: '模型别名（选填）' })}
          </label>
          <Input
            id={modelAliasInputId}
            value={value.displayName ?? ''}
            placeholder={t('settings.modelDisplayNamePlaceholder', { defaultValue: '例如：Seedance 1.5 Pro' })}
            maxLength={128}
            allowClear
            onChange={(displayName) => onChange((current) => ({ ...current, displayName }))}
            aria-describedby={`${modelAliasInputId}-hint`}
            data-model-alias-input
          />
          <div id={`${modelAliasInputId}-hint`} className='text-11px leading-4 text-t-tertiary'>
            {t('settings.modelDisplayNameHint', { defaultValue: '非必填，仅用于界面展示；实际请求仍使用原始模型 ID。' })}
          </div>
        </div>
      )}

      {capabilityTask === undefined && value.capabilities.length === 0 && (
        <section hidden={focusedCallConfigTask !== undefined} className='space-y-8px' data-model-purpose-required>
          <div className='text-13px font-500 text-t-secondary'>
            {t('settings.modelPurpose', { defaultValue: '调用用途' })}
          </div>
          <Select
            value={undefined}
            options={MODEL_TASK_ORDER.map((task) => ({ value: task, label: t(`settings.modelTask.${task}`, { defaultValue: task }) }))}
            placeholder={t('settings.selectModelPurpose', { defaultValue: '确认模型的调用用途' })}
            aria-label={t('settings.modelPurpose', { defaultValue: '调用用途' })}
            status={value.model.trim() ? 'error' : undefined}
            onChange={addCallRoute}
            triggerProps={{ getPopupContainer: () => document.body }}
            data-model-purpose-picker
          />
          <div className='text-11px leading-4 text-t-secondary' role='note'>
            {selectedCatalogEntry?.tasksSource === 'inferred' && selectedCatalogEntry.tasks.length > 0
              ? t('settings.modelPurposeUnverifiedHint', { defaultValue: '目录中的用途只是推测，请确认实际用途；此选择不会筛选模型列表。' })
              : t('settings.modelPurposeRequiredHint', { defaultValue: '用途决定调用接口与请求格式。未知模型不会自动按对话处理，模型 ID 仍可自由输入。' })}
          </div>
        </section>
      )}

      {taskConflict && (
        <div hidden={focusedCallConfigTask !== undefined} className='space-y-8px rounded-8px border border-solid border-warning-4 bg-warning-1 p-12px' role={taskConflict.acknowledged ? 'note' : 'alert'} data-model-purpose-conflict>
          <div className='text-12px leading-4 text-warning-7'>
            {t('settings.modelPurposeConflict', {
              configured: taskConflict.configuredTasks.map((task) => t(`settings.modelTask.${task}`, { defaultValue: task })).join('、'),
              declared: taskConflict.declaredTasks.map((task) => t(`settings.modelTask.${task}`, { defaultValue: task })).join('、'),
              defaultValue: '当前配置用于 {{configured}}，目录明确列出的用途为 {{declared}}。请确认所选模型支持当前接口。',
            })}
          </div>
          {taskConflict.acknowledged
            ? <div className='text-11px text-t-secondary'>{t('settings.modelPurposeConflictAcknowledged', { defaultValue: '已确认保留当前用途，调用仍使用对应接口。' })}</div>
            : <Button size='small' onClick={() => onChange(acknowledgeCatalogTaskConflict)} data-confirm-model-purpose>
                {t('settings.confirmModelPurpose', { defaultValue: '确认保留当前调用用途' })}
              </Button>}
        </div>
      )}

      <div className='space-y-10px' data-capability-card-list>
        {value.capabilities
          .filter(
            (capability) =>
              (capabilityTask === undefined || capability.task === capabilityTask) &&
              (focusedCallConfigTask === undefined || capability.task === focusedCallConfigTask)
          )
          .map((capability) => {
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
        const parsedProviderParams = parseProviderParams(capability.providerParamsJson);
        const providerParamsValid = parsedProviderParams.ok;
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
          descriptor &&
            manifest &&
            descriptor.protocol_id !== recommended &&
            !descriptor.platforms.includes(manifest.platform)
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
          !isValidModelTokenLimit(capability.outputLimit);
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
        const callConfigIntent = callConfigIntentByTask[capability.task] ?? 'overview';
        const callConfigBaseline = callConfigBaselineByTask[capability.task];
        const callConfigChanged =
          callConfigBaseline !== undefined && !sameCapabilityDraft(callConfigBaseline, capability);
        const contextLimitSummary = compactTokenCount(
          capability.contextLimit,
          t('settings.modelAdvanced.contextDefaultCompact', { defaultValue: '自动' })
        );
        const outputLimitSummary = compactTokenCount(
          capability.outputLimit,
          t('settings.modelAdvanced.outputProviderDefaultCompact', {
            defaultValue: '由供应商决定',
          })
        );
        const outputLimitEditorOpen =
          Boolean(editingOutputLimitByTask[capability.task]) || outputLimitMissing;
        const reasoningEffort = providerParamReasoningEffort(capability.providerParamsJson);
        const reasoningEffortOptions = reasoningEffortsForProtocol(capability.protocol);
        const hasReasoningEffort =
          parsedProviderParams.ok &&
          Object.prototype.hasOwnProperty.call(parsedProviderParams.value, 'reasoning_effort');
        const reasoningEffortProtocolSupported = protocolSupportsReasoningEffort(capability.protocol);
        const reasoningEffortAvailable =
          capability.task === 'chat' &&
          reasoningEffortProtocolSupported &&
          providerParamsValid;
        const protocolTransportOpen =
          protocolTransportOpenByTask[capability.task] ??
          taskValidationErrors.some((error) =>
            [
              'base_url_required',
              'cross_origin_consent_required',
              'connection_missing',
            ].includes(error.code)
          );
        const protocolParamsOpen =
          protocolParamsOpenByTask[capability.task] ??
          (Boolean(capability.providerParamsJson.trim()) ||
            taskValidationErrors.some((error) => error.code === 'invalid_provider_params'));
        const primaryEndpointField = [...endpointFields][0];
        const primaryEndpointDescriptor: CapabilityEndpointDescriptor | undefined = primaryEndpointField
          ? endpointDescriptors.find((endpoint) => endpoint.field === primaryEndpointField) ?? {
              task: capability.task,
              field: primaryEndpointField,
              purpose: 'submit' as const,
              method: null,
              default_value: '',
              root_shape: 'versioned_root' as const,
              allowed_placeholders: [],
              required_placeholders: [],
              editable: true,
            }
          : undefined;
        const actualRequestUrl =
          primaryEndpointDescriptor && !sdkTransport
            ? resolvedCapabilityUrl(
                capability,
                primaryEndpointDescriptor,
                manifest,
                providerBaseUrl,
                connections
              )
            : '';
        const callConfigIntents: Array<{
          key: CallConfigIntent;
          title: string;
          description: string;
          icon: React.ReactNode;
        }> = [
          {
            key: 'overview',
            title: t('settings.modelAdvanced.intentOverview', {
              defaultValue: '概览',
            }),
            description: t('settings.modelAdvanced.intentOverviewHint', {
              defaultValue: '查看系统已安装的当前方案',
            }),
            icon: <CheckOne theme='outline' size='17' />,
          },
          {
            key: 'connection',
            title: t('settings.modelAdvanced.intentConnection', {
              defaultValue: '使用另一套连接',
            }),
            description: t('settings.modelAdvanced.intentConnectionHint', {
              defaultValue: '选择当前供应商下已有的连接档案',
            }),
            icon: <LinkOne theme='outline' size='17' />,
          },
          {
            key: 'limits',
            title: capability.task === 'chat'
              ? t('settings.modelAdvanced.reasoningEffort', { defaultValue: '思考深度' })
              : t('settings.modelAdvanced.intentLimits', { defaultValue: '生成与限制' }),
            description: capability.task === 'chat'
              ? t('settings.modelAdvanced.reasoningEffortDescription', { defaultValue: '调整模型默认的思考深度。' })
              : t('settings.modelAdvanced.intentLimitsHint', { defaultValue: '设置上下文与最大输出限制' }),
            icon: <Shield theme='outline' size='17' />,
          },
          {
            key: 'protocol',
            title: t('settings.modelAdvanced.intentProtocol', {
              defaultValue: '兼容特殊调用方式',
            }),
            description: t('settings.modelAdvanced.intentProtocolHint', {
              defaultValue: '更改协议、Base URL 或 Endpoint',
            }),
            icon: <Code theme='outline' size='17' />,
          },
          {
            key: 'diagnostics',
            title: t('settings.modelAdvanced.intentDiagnostics', {
              defaultValue: '排查请求问题',
            }),
            description: t('settings.modelAdvanced.intentDiagnosticsHint', {
              defaultValue: '查看实际请求、鉴权兼容性与错误定位',
            }),
            icon: <Search theme='outline' size='17' />,
          },
        ];

        return (
          <section
            key={capability.task}
            className={
              focusedCallConfigTask === capability.task
                ? 'overflow-hidden'
                : `overflow-hidden rounded-8px border border-solid ${
                    hasValidationError ? 'border-danger-4' : 'border-[var(--color-border-2)]'
                  }`
            }
            data-capability-card={capability.task}
            data-capability-has-error={hasValidationError}
            data-capability-expanded={expanded}
          >
            <div
              className='flex items-stretch'
              style={
                focusedCallConfigTask === capability.task ? { display: 'none' } : undefined
              }
              data-capability-card-header={capability.task}
            >
              <div className='w-full space-y-6px px-14px py-12px'>
                <div className='flex min-w-0 items-center gap-8px'>
                  <span className='font-600 text-t-primary'>
                    {t(`settings.modelTask.${capability.task}`, { defaultValue: capability.task })}
                  </span>
                  <Tag size='small' color={statusColor}>{statusText}</Tag>
                  <div className='ml-auto flex shrink-0 items-center gap-6px'>
                    <Button
                      size='mini'
                      type='secondary'
                      aria-expanded={expanded}
                      aria-controls={detailsId}
                      aria-label={t('settings.modelAdvanced.advancedForTask', {
                        task: t(`settings.modelTask.${capability.task}`, { defaultValue: capability.task }),
                        defaultValue: '{{task}}高级配置',
                      })}
                      data-capability-disclosure={capability.task}
                      onClick={() => toggleCallConfig(capability.task)}
                    >
                      {expanded
                        ? t('settings.modelAdvanced.collapseConfiguration', { defaultValue: '收起高级配置' })
                        : t('settings.modelAdvanced.advancedConfiguration', { defaultValue: '高级配置' })}
                      {expanded ? <Down theme='outline' size='12' /> : <Right theme='outline' size='12' />}
                    </Button>
                    {capabilityTask === undefined && <Popconfirm
                      title={t('settings.removeModelTaskConfirm', {
                        defaultValue: `移除“${t(`settings.modelTask.${capability.task}`, { defaultValue: capability.task })}”接口及其调用配置？`,
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
                        aria-label={t('settings.removeModelTask', { defaultValue: '移除调用接口' })}
                        data-remove-model-task={capability.task}
                      />
                    </Popconfirm>}
                  </div>
                </div>
                <div
                  className='flex min-w-0 flex-wrap items-center gap-x-6px gap-y-2px text-11px text-t-secondary'
                  data-capability-summary={capability.task}
                >
                  <span className='max-w-260px truncate' title={protocolSummary}>{protocolSummary}</span>
                  <span aria-hidden='true'>·</span>
                  <span>{selectedRole}</span>
                  <span aria-hidden='true'>·</span>
                  <span className='max-w-300px truncate' title={baseUrlSummary}>{baseUrlSummary}</span>
                </div>
              </div>
            </div>
            {/*
              What is actually missing, in words, outside the disclosure.
              The card used to state only a count ("待处理 1 项") while the
              offending control showed a bare red border, so a `new-api`
              provider — which legitimately requires an explicit protocol per
              model — reported "incomplete" with nothing to act on. Rendered
              above the fold because a collapsed card must still say why it
              blocks the save.
            */}
            {hasValidationError && (
              <div
                className='space-y-4px border-0 border-t border-solid border-[var(--color-border-2)] bg-danger-1 px-14px py-10px'
                role='alert'
                data-capability-error-list={capability.task}
              >
                {taskValidationErrors.map((error) => (
                  <div
                    key={error.code}
                    className='text-11px leading-4 text-danger-6'
                    data-capability-error={error.code}
                  >
                    {t(capabilityValidationMessageKey(error.code), { defaultValue: error.code })}
                  </div>
                ))}
              </div>
            )}

            {capability.task === 'chat' && (
              <div
                hidden={focusedCallConfigTask !== undefined}
                className='space-y-10px border-0 border-t border-solid border-[var(--color-border-2)] px-14px py-12px'
                data-model-context-settings={capability.task}
              >
                <div className='text-12px font-600 text-t-primary'>
                  {t('settings.modelAdvanced.contextSettingsTitle', { defaultValue: '上下文与输出' })}
                </div>
                <div className='grid grid-cols-2 gap-10px'>
                  <div className='space-y-6px'>
                    <div className='text-12px text-t-secondary'>
                      {t('settings.contextLimit', { defaultValue: '上下文窗口（tokens）' })}
                    </div>
                    <ContextLimitSelect
                      value={capability.contextLimit}
                      onChange={(contextLimit) => updateCapability(capability.task, { contextLimit })}
                    />
                    <div className='text-11px leading-4 text-t-tertiary'>
                      {t('settings.modelAdvanced.contextLimitCompactHint', {
                        defaultValue: '未填写不覆盖模型上下文。仅采用供应商明确提供的窗口；未知时请按模型文档填写，安全预算独立。',
                      })}
                    </div>
                  </div>
                  <div className='space-y-6px' data-model-output-settings={capability.task}>
                    <div className='text-12px text-t-secondary'>
                      {t('settings.outputLimit', { defaultValue: '最大输出（tokens）' })}
                    </div>
                    <OutputLimitInput
                      value={capability.outputLimit}
                      onChange={(outputLimit) => updateCapability(capability.task, { outputLimit })}
                      compact
                    />
                    {outputLimitMissing && (
                      <div className='text-11px text-danger-6' role='alert' data-output-limit-required>
                        {t('settings.outputLimitRequired')}
                      </div>
                    )}
                  </div>
                </div>
                  <div className='space-y-6px'>
                    <div className='text-12px text-t-secondary'>
                      {t('settings.modelAdvanced.compactionThresholdLabel', {
                        defaultValue: '自动压缩阈值',
                      })}
                    </div>
                    <Select
                      value={capability.compactionThresholdPct ?? 75}
                      options={[50, 60, 70, 75, 80, 85, 90, 95].map((pct) => ({
                        value: pct,
                        label: `${pct}%${pct === 75 ? ` · ${t('settings.modelAdvanced.recommended', { defaultValue: '推荐' })}` : ''}`,
                      }))}
                      onChange={(compactionThresholdPct: number) =>
                        updateCapability(capability.task, {
                          compactionThresholdPct: compactionThresholdPct === 75 ? undefined : compactionThresholdPct,
                        })
                      }
                      getPopupContainer={() => document.body}
                      aria-label={t('settings.modelAdvanced.compactionThresholdLabel', {
                        defaultValue: '自动压缩阈值',
                      })}
                      data-model-compaction-threshold
                    />
                    <div className='text-11px leading-4 text-t-tertiary'>
                      {t('settings.modelAdvanced.compactionThresholdHint', {
                        defaultValue: '达到可用输入空间的这个比例时自动压缩；调低会更早压缩。',
                      })}
                    </div>
                  </div>
              </div>
            )}

            <div
              id={detailsId}
              hidden={!expanded}
              className={
                focusedCallConfigTask === capability.task
                  ? 'space-y-12px'
                  : 'space-y-12px border-0 border-t border-solid border-[var(--color-border-2)] p-14px'
              }
              data-capability-details={capability.task}
            >

            <div
              className='flex min-w-0 flex-wrap items-center gap-x-8px gap-y-2px text-11px text-t-secondary'
              data-call-config-context={capability.task}
            >
              {focusedCallConfigTask === capability.task && (
                <button
                  type='button'
                  className='mr-2px inline-flex size-26px shrink-0 items-center justify-center rounded-7px border-0 bg-transparent text-t-secondary hover:bg-fill-2 hover:text-t-primary'
                  aria-label={t('settings.modelAdvanced.backToModel', {
                    defaultValue: '返回模型',
                  })}
                  onClick={() => finishCallConfig(capability.task, false)}
                >
                  <Left theme='outline' size='14' />
                </button>
              )}
              {providerLabel && <span className='font-500 text-t-primary'>{providerLabel}</span>}
              {providerLabel && <span aria-hidden='true'>/</span>}
              <span>{value.model}</span>
              <span aria-hidden='true'>/</span>
              <span>
                {t(`settings.modelTask.${capability.task}`, {
                  defaultValue: capability.task,
                })}
              </span>
            </div>

            <div
              className={`flex items-start gap-10px rounded-10px px-12px py-8px ${
                hasValidationError ? 'bg-danger-1 text-danger-7' : 'bg-success-1 text-success-7'
              }`}
              data-call-config-summary={capability.task}
            >
              <CheckOne theme='outline' size='18' className='mt-1px shrink-0' />
              <div className='min-w-0 flex-1'>
                <div className='text-13px font-600 leading-18px'>
                  {hasValidationError
                    ? t('settings.modelAdvanced.callConfigNeedsAttention', {
                        defaultValue: '当前配置需要处理',
                      })
                    : t('settings.modelAdvanced.callConfigReady', {
                        defaultValue: '当前推荐配置可直接使用',
                      })}
                </div>
                <div className='mt-2px flex min-w-0 flex-wrap gap-x-6px gap-y-2px text-11px text-t-secondary'>
                  <span>{protocolSummary}</span>
                  <span aria-hidden='true'>·</span>
                  <span>{selectedRole}</span>
                  <span aria-hidden='true'>·</span>
                  <span>{contextLimitSummary}</span>
                </div>
              </div>
            </div>

            <div
              className='overflow-x-auto border-0 border-b border-solid border-[var(--color-border-2)]'
              role='tablist'
              aria-label={t('settings.modelAdvanced.callConfigurationTitle', {
                defaultValue: '调用配置',
              })}
              data-call-config-tabs={capability.task}
            >
              <div className='flex min-w-max items-end gap-2px'>
                {callConfigIntents.map((intent) => {
                  const selected = callConfigIntent === intent.key;
                  return (
                    <Tooltip key={intent.key} content={intent.description} position='top'>
                      <button
                        type='button'
                        role='tab'
                        aria-selected={selected}
                        className={`inline-flex h-38px min-w-112px items-center justify-center gap-6px whitespace-nowrap border-0 border-b-2px border-b-solid bg-transparent px-12px text-12px transition-colors focus-visible:outline-none ${
                          selected
                            ? 'border-primary-6 text-primary-6'
                            : 'border-transparent text-t-secondary hover:bg-fill-1 hover:text-t-primary'
                        }`}
                        data-call-config-tab={intent.key}
                        onClick={() =>
                          setCallConfigIntentByTask((current) => ({
                            ...current,
                            [capability.task]: intent.key,
                          }))
                        }
                      >
                        {intent.icon}
                        <span>{intent.title}</span>
                      </button>
                    </Tooltip>
                  );
                })}
              </div>
            </div>

            <div
              hidden={callConfigIntent !== 'overview'}
              className='flex items-start gap-10px rounded-8px bg-fill-1 px-12px py-10px'
              data-call-config-branch='overview'
            >
              <CheckOne theme='outline' size='17' className='mt-1px shrink-0 text-success-6' />
              <div className='min-w-0'>
                <div className='text-12px font-600 text-t-primary'>
                  {t('settings.modelAdvanced.overviewTitle', {
                    defaultValue: '系统方案已安装',
                  })}
                </div>
                <div className='mt-2px text-11px leading-16px text-t-secondary'>
                  {t('settings.modelAdvanced.overviewHint', {
                    defaultValue:
                      '当前协议、连接和请求路径可直接使用。仅在需要覆盖默认值时切换上方页签。',
                  })}
                </div>
              </div>
            </div>

            <div
              hidden={callConfigIntent !== 'diagnostics'}
              className='space-y-10px rounded-10px border border-solid border-[var(--color-border-2)] p-14px'
              data-call-config-branch='diagnostics'
            >
              <div>
                <div className='text-13px font-600 text-t-primary'>
                  {t('settings.modelAdvanced.diagnosticsTitle', { defaultValue: '请求诊断' })}
                </div>
                <div className='mt-3px text-11px leading-17px text-t-secondary'>
                  {t('settings.modelAdvanced.diagnosticsHint', {
                    defaultValue: '先查看系统解析出的请求；需要修改时再进入对应目标。',
                  })}
                </div>
              </div>
              <div className='divide-y divide-[var(--color-border-2)] rounded-8px border border-solid border-[var(--color-border-2)]'>
                <div className='grid grid-cols-[116px_minmax(0,1fr)] gap-10px px-12px py-9px text-12px'>
                  <span className='text-t-tertiary'>
                    {t('settings.modelAdvanced.protocol', { defaultValue: '调用协议' })}
                  </span>
                  <span className='text-t-primary'>{protocolSummary}</span>
                </div>
                <div className='grid grid-cols-[116px_minmax(0,1fr)] gap-10px px-12px py-9px text-12px'>
                  <span className='text-t-tertiary'>
                    {t('settings.modelAdvanced.connectionRole', { defaultValue: '连接档案' })}
                  </span>
                  <span className='text-t-primary'>{selectedRole}</span>
                </div>
                <div className='grid grid-cols-[116px_minmax(0,1fr)] gap-10px px-12px py-9px text-12px'>
                  <span className='text-t-tertiary'>
                    {t('settings.modelAdvanced.resolvedUrl', { defaultValue: '实际请求地址' })}
                  </span>
                  <span className='break-all text-t-primary'>
                    {actualRequestUrl || baseUrlSummary}
                  </span>
                </div>
                <div className='grid grid-cols-[116px_minmax(0,1fr)] gap-10px px-12px py-9px text-12px'>
                  <span className='text-t-tertiary'>
                    {t('settings.authScheme', { defaultValue: '鉴权方式' })}
                  </span>
                  <span className={authSchemeCompatible ? 'text-t-primary' : 'text-danger-6'}>
                    {selectedAuthScheme || t('common.unknown', { defaultValue: '未知' })}
                  </span>
                </div>
              </div>
              {taskValidationErrors.length > 0 && (
                <div className='space-y-4px rounded-8px bg-danger-1 p-10px' role='alert'>
                  {taskValidationErrors.map((error) => (
                    <div key={error.code} className='text-11px leading-16px text-danger-6'>
                      {t(capabilityValidationMessageKey(error.code), { defaultValue: error.code })}
                    </div>
                  ))}
                </div>
              )}
              <div className='flex flex-wrap gap-8px'>
                <Button
                  size='small'
                  onClick={() =>
                    setCallConfigIntentByTask((current) => ({
                      ...current,
                      [capability.task]: 'connection',
                    }))
                  }
                >
                  {t('settings.modelAdvanced.intentConnection', {
                    defaultValue: '使用另一套连接',
                  })}
                </Button>
                <Button
                  size='small'
                  onClick={() =>
                    setCallConfigIntentByTask((current) => ({
                      ...current,
                      [capability.task]: 'protocol',
                    }))
                  }
                >
                  {t('settings.modelAdvanced.intentProtocol', {
                    defaultValue: '兼容特殊调用方式',
                  })}
                </Button>
              </div>
            </div>

            <div hidden={callConfigIntent !== 'protocol'} className='space-y-6px' data-call-config-branch='protocol'>
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
                  const nextProtocol = typeof protocol === 'string' ? protocol : '';
                  onChange((current) => {
                    const selected = current.capabilities.find(
                      (candidate) => candidate.task === capability.task
                    );
                    if (!selected) return current;
                    const nextCapability = changeCapabilityProtocol(
                      selected,
                      nextProtocol,
                      manifest
                    );
                    return nextCapability === selected
                      ? current
                      : {
                          ...current,
                          capabilities: current.capabilities.map((candidate) =>
                            candidate.task === capability.task ? nextCapability : candidate
                          ),
                        };
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
              {!sdkTransport && (
                <button
                  type='button'
                  className='flex w-full items-center gap-8px rounded-8px border border-solid border-[var(--color-border-2)] bg-fill-1 px-10px py-8px text-left hover:bg-fill-2'
                  aria-expanded={protocolTransportOpen}
                  data-protocol-transport-disclosure={capability.task}
                  onClick={() =>
                    setProtocolTransportOpenByTask((current) => ({
                      ...current,
                      [capability.task]: !protocolTransportOpen,
                    }))
                  }
                >
                  <span className='min-w-0 flex-1'>
                    <span className='block text-12px font-500 text-t-primary'>
                      {t('settings.modelAdvanced.addressOverrides', {
                        defaultValue: '地址与 Endpoint 覆盖',
                      })}
                    </span>
                    <span className='mt-1px block truncate text-11px text-t-tertiary'>
                      {actualRequestUrl || baseUrlSummary}
                    </span>
                  </span>
                  <span className='shrink-0 text-11px text-t-secondary'>
                    {protocolTransportOpen
                      ? t('common.collapse', { defaultValue: '收起' })
                      : t('common.edit', { defaultValue: '修改' })}
                  </span>
                  {protocolTransportOpen ? (
                    <Down theme='outline' size='13' className='shrink-0 text-t-tertiary' />
                  ) : (
                    <Right theme='outline' size='13' className='shrink-0 text-t-tertiary' />
                  )}
                </button>
              )}
            </div>

            {/*
              One container for the whole transport chain, nested in the order
              the code actually resolves it (`effectiveBaseUrl`): the connection
              profile owns the URL, auth scheme and credentials; a per-task Base
              URL override replaces only the URL half of that profile; endpoints
              are relative paths joined onto whichever URL won. It used to render
              level 2 before level 1, which is why the relationship read as
              arbitrary.
            */}
            <div
              hidden={
                callConfigIntent !== 'connection' &&
                (callConfigIntent !== 'protocol' || !protocolTransportOpen)
              }
              className='space-y-10px rounded-8px border border-solid border-[var(--color-border-2)] p-12px'
              data-transport-group={capability.task}
              data-call-config-branch={callConfigIntent}
            >
              <div className='text-12px font-500 text-t-secondary'>
                {callConfigIntent === 'connection'
                  ? t('settings.modelAdvanced.connectionBranchTitle', {
                      defaultValue: '供应商连接',
                    })
                  : t('settings.modelAdvanced.protocolBranchTitle', {
                      defaultValue: '协议、地址与 Endpoint',
                    })}
              </div>

              <div hidden={callConfigIntent !== 'connection'} className='space-y-6px'>
                <div className='text-12px text-t-secondary'>
                  {t('settings.modelAdvanced.connectionRole', { defaultValue: '连接档案' })}
                </div>
                <div className='text-11px leading-4 text-t-tertiary'>
                  {t('settings.modelAdvanced.connectionRoleHint', {
                    defaultValue: '决定这次请求用哪套地址、鉴权方式和凭据。',
                  })}
                </div>
                <div className='rounded-8px bg-fill-1 px-10px py-8px text-11px leading-16px text-t-secondary'>
                  {t('settings.modelAdvanced.connectionOwnershipHint', {
                    defaultValue: '连接档案归当前供应商管理；这里只为当前任务选择引用。',
                  })}
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
                    {t('settings.modelAdvanced.createProviderConnection', {
                      defaultValue: '添加供应商连接档案',
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

              {!sdkTransport && (
                <div
                  hidden={callConfigIntent !== 'protocol'}
                  className='space-y-6px border-0 border-l border-solid border-[var(--color-border-2)] pl-12px'
                  data-transport-level='base-url'
                >
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
                          defaultValue: '继承上方连接档案的地址；勾选后才会写入任务级覆盖。',
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

            {endpointFields.size > 0 && (
              <div
                hidden={callConfigIntent !== 'protocol'}
                className='space-y-10px border-0 border-l border-solid border-[var(--color-border-2)] pl-12px'
                data-transport-level='endpoints'
              >
                <div className='text-11px leading-4 text-t-tertiary'>
                  {t('settings.modelAdvanced.endpointsHint', {
                    defaultValue: '相对路径，拼在上方生效的 Base URL 之后。',
                  })}
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
              </div>
            )}

            {crossOrigin && (
              <div
                hidden={callConfigIntent !== 'protocol'}
                className='rounded-8px bg-warning-1 p-10px space-y-6px'
                data-cross-origin-consent
              >
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
            </div>

            {/* Chat context and output are on the model homepage. Advanced
                Chat settings tune reasoning; other tasks keep their limits here. */}
            <div
              hidden={callConfigIntent !== 'limits'}
              className='space-y-10px rounded-10px border border-solid border-[var(--color-border-2)] p-12px'
              data-token-limits
              data-call-config-branch='limits'
            >
              <div className='text-12px font-500 text-t-secondary'>
                {capability.task === 'chat'
                  ? t('settings.modelAdvanced.reasoningEffort', { defaultValue: '思考深度' })
                  : t('settings.modelAdvanced.modelLimitsTitle', { defaultValue: '生成与限制' })}
              </div>

              {capability.task === 'chat' && (
                <div className='space-y-8px' data-reasoning-effort-control>
                  <div className='flex flex-wrap items-center justify-between gap-10px'>
                    <div>
                      <div className='text-12px text-t-secondary'>
                        {t('settings.modelAdvanced.reasoningEffort', { defaultValue: '思考深度' })}
                      </div>
                      <div className='mt-2px text-11px leading-4 text-t-tertiary'>
                        {t('settings.modelAdvanced.reasoningEffortDescription', {
                          defaultValue: '作为该模型 Chat 能力的默认值，影响速度、质量和 token 消耗。',
                        })}
                      </div>
                    </div>
                    <div
                      className='inline-flex overflow-hidden rounded-8px border border-solid border-[var(--color-border-2)] bg-fill-1'
                      role='group'
                      aria-label={t('settings.modelAdvanced.reasoningEffort', { defaultValue: '思考深度' })}
                    >
                      {([
                        { value: undefined, key: 'auto' },
                        ...reasoningEffortOptions.map((value) => ({ value, key: value })),
                      ] satisfies Array<{ value: ModelReasoningEffort | undefined; key: string }>).map((option) => {
                        const selected = option.value === undefined
                          ? !hasReasoningEffort
                          : reasoningEffort === option.value;
                        const disabled = !providerParamsValid || (option.value !== undefined && !reasoningEffortAvailable);
                        return (
                          <button
                            key={option.key}
                            type='button'
                            aria-pressed={selected}
                            disabled={disabled}
                            data-reasoning-effort={option.key}
                            className={`min-w-58px border-0 border-r border-solid border-[var(--color-border-2)] px-11px py-6px text-12px last:border-r-0 disabled:cursor-not-allowed disabled:opacity-45 ${
                              selected
                                ? 'bg-primary-1 text-primary-6'
                                : 'bg-transparent text-t-secondary hover:bg-fill-2'
                            }`}
                            onClick={() =>
                              updateCapability(capability.task, {
                                providerParamsJson: withProviderParamReasoningEffort(
                                  capability.providerParamsJson,
                                  option.value
                                ),
                              })
                            }
                          >
                            {t(`settings.modelAdvanced.reasoningEffortOptions.${option.key}`, {
                              defaultValue: option.key,
                            })}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                  <div
                    className={`text-11px leading-4 ${
                      providerParamsValid && reasoningEffortProtocolSupported
                        ? 'text-t-tertiary'
                        : 'text-warning-7'
                    }`}
                    data-reasoning-effort-hint
                  >
                    {!providerParamsValid
                      ? t('settings.modelAdvanced.reasoningEffortJsonInvalid', {
                          defaultValue: '先修正供应商参数 JSON，才能调整思考深度。',
                        })
                      : !reasoningEffortProtocolSupported
                          ? t('settings.modelAdvanced.reasoningEffortProtocolUnsupported', {
                              defaultValue: '当前协议不提供统一的低/中/高映射；请使用自动。',
                            })
                          : t('settings.modelAdvanced.reasoningEffortHint', {
                              defaultValue: '自动使用系统与供应商默认值；固定档位会应用到使用该模型的新调用。',
                            })}
                  </div>
                </div>
              )}

              {capability.task !== 'chat' && (
                <div className='space-y-8px'>
                  <div className='text-12px text-t-secondary'>
                    {t('settings.contextLimit', { defaultValue: '上下文窗口（tokens）' })}
                  </div>
                  <ContextLimitSelect
                    value={capability.contextLimit}
                    onChange={(contextLimit) => updateCapability(capability.task, { contextLimit })}
                  />
                </div>
              )}

              {capability.task !== 'chat' && <div className='space-y-8px'>
                <div className='flex items-center justify-between gap-10px rounded-8px bg-fill-1 px-10px py-8px'>
                  <span className='text-12px text-t-secondary'>
                    {t('settings.outputLimit', { defaultValue: '最大输出（tokens）' })}
                  </span>
                  <span className='ml-auto text-12px text-t-primary'>{outputLimitSummary}</span>
                  <Button
                    size='mini'
                    type='text'
                    onClick={() =>
                      setEditingOutputLimitByTask((current) => ({
                        ...current,
                        [capability.task]: !outputLimitEditorOpen,
                      }))
                    }
                  >
                    {capability.outputLimit === undefined
                      ? t('settings.modelAdvanced.setOutputLimit', {
                          defaultValue: '设置上限',
                        })
                      : t('common.edit', { defaultValue: '更改' })}
                  </Button>
                </div>
                <div hidden={!outputLimitEditorOpen}>
                  <OutputLimitInput
                    value={capability.outputLimit}
                    onChange={(outputLimit) =>
                      updateCapability(capability.task, { outputLimit })
                    }
                    compact
                  />
                </div>
                {outputLimitMissing && (
                  <div className='text-11px text-danger-6' role='alert' data-output-limit-required>
                    {t('settings.outputLimitRequired', {
                      defaultValue: 'This protocol requires a numeric max output value. No provider/model recommendation is available; enter the documented value instead of guessing.',
                    })}
                  </div>
                )}
              </div>}
              <div className='flex items-center justify-between gap-10px border-0 border-t border-solid border-[var(--color-border-2)] pt-10px'>
                <span className='text-11px leading-16px text-t-secondary'>
                  {t('settings.modelAdvanced.taskOnlyHint', {
                    model: value.model,
                    task: t(`settings.modelTask.${capability.task}`, {
                      defaultValue: capability.task,
                    }),
                    defaultValue: '只保存到当前模型任务，不影响其他任务。',
                  })}
                </span>
                <Button
                  size='mini'
                  type='text'
                  onClick={() =>
                    updateCapability(capability.task, {
                      ...(capability.task === 'chat' ? {} : { contextLimit: undefined, outputLimit: undefined }),
                      providerParamsJson: withProviderParamReasoningEffort(
                        capability.providerParamsJson,
                        undefined
                      ),
                    })
                  }
                >
                  {t('settings.restoreProtocolDefault', { defaultValue: '恢复推荐值' })}
                </Button>
              </div>
            </div>

            <div hidden={callConfigIntent !== 'protocol'} className='space-y-8px'>
            <button
              type='button'
              className='flex w-full items-center gap-8px rounded-8px border border-solid border-[var(--color-border-2)] bg-fill-1 px-10px py-8px text-left hover:bg-fill-2'
              aria-expanded={protocolParamsOpen}
              data-provider-params-disclosure={capability.task}
              onClick={() =>
                setProtocolParamsOpenByTask((current) => ({
                  ...current,
                  [capability.task]: !protocolParamsOpen,
                }))
              }
            >
              <span className='min-w-0 flex-1'>
                <span className='block text-12px font-500 text-t-primary'>
                  {t('settings.modelAdvanced.providerOptions', {
                    defaultValue: '供应商专用参数',
                  })}
                </span>
                <span className='mt-1px block text-11px text-t-tertiary'>
                  {capability.providerParamsJson.trim()
                    ? t('settings.modelAdvanced.providerOptionsConfigured', {
                        defaultValue: '已设置自定义参数',
                      })
                    : t('settings.modelAdvanced.providerOptionsDefault', {
                        defaultValue: '当前使用协议默认值',
                      })}
                </span>
              </span>
              <span className='shrink-0 text-11px text-t-secondary'>
                {protocolParamsOpen
                  ? t('common.collapse', { defaultValue: '收起' })
                  : t('common.edit', { defaultValue: '修改' })}
              </span>
              {protocolParamsOpen ? (
                <Down theme='outline' size='13' className='shrink-0 text-t-tertiary' />
              ) : (
                <Right theme='outline' size='13' className='shrink-0 text-t-tertiary' />
              )}
            </button>
            <div hidden={!protocolParamsOpen} className='space-y-12px'>
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
                      defaultValue: '只填写供应商原始参数；切换协议会保留自定义值，不兼容的参数需要明确修正。URL 和凭据边界独立管理。',
                    })
                  : t('settings.modelAdvanced.invalidParamsJson', {
                      defaultValue: '必须是合法的 JSON 对象。',
                    })}
              </div>
            </div>
            </div>
            </div>

            {callConfigBaseline && callConfigFooterPlacement === 'internal' && (
              <div className='flex flex-wrap items-center justify-between gap-10px border-0 border-t border-solid border-[var(--color-border-2)] pt-12px' data-call-config-footer={capability.task}>
                <span className='text-11px text-t-secondary' aria-live='polite'>
                  {callConfigChanged
                    ? t('settings.modelAdvanced.pendingDraftChange', {
                        defaultValue: '更改尚未保存到模型。',
                      })
                    : t('settings.modelAdvanced.noDraftChange', {
                        defaultValue: '尚未更改',
                      })}
                </span>
                <div className='flex gap-8px'>
                  <Button size='small' onClick={() => finishCallConfig(capability.task, true)}>
                    {t('settings.modelAdvanced.cancelAdjustment', {
                      defaultValue: '取消调整',
                    })}
                  </Button>
                  <Button
                    type='primary'
                    size='small'
                    onClick={() => finishCallConfig(capability.task, false)}
                  >
                    {t('settings.modelAdvanced.applyToTask', {
                      task: t(`settings.modelTask.${capability.task}`, {
                        defaultValue: capability.task,
                      }),
                      defaultValue: '应用到当前任务',
                    })}
                  </Button>
                </div>
              </div>
            )}
            </div>
          </section>
        );
      })}
      </div>
      {capabilityTask === undefined && <div hidden={focusedCallConfigTask !== undefined} className='space-y-8px' data-model-call-routes>
        {selectedTasks.length > 0 && selectedTasks.length < MODEL_TASK_ORDER.length && (
          <>
            <Button type='text' size='small' onClick={() => setAddingCallRoute((open) => !open)} aria-expanded={addingCallRoute} data-add-call-route>
              {t('settings.addModelCallRoute', { defaultValue: '添加图像、语音等调用接口' })}
            </Button>
            {addingCallRoute && (
              <Select
                value={undefined}
                options={MODEL_TASK_ORDER.filter((task) => !selectedTasks.includes(task)).map((task) => ({
                  value: task,
                  label: t(`settings.modelTask.${task}`, { defaultValue: task }),
                }))}
                placeholder={t('settings.selectModelCallRoute', { defaultValue: '选择需要调用的接口' })}
                aria-label={t('settings.selectModelCallRoute', { defaultValue: '选择需要调用的接口' })}
                onChange={addCallRoute}
                triggerProps={{ getPopupContainer: () => document.body }}
                data-model-call-route-picker
              />
            )}
          </>
        )}
      </div>}
    </div>
  );
});

ModelDefinitionEditor.displayName = 'ModelDefinitionEditor';

export default ModelDefinitionEditor;
