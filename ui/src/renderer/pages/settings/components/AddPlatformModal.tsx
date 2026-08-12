/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IProvider } from '@/common/config/storage';
import { parseProviderId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import NomiModal from '@/renderer/components/base/NomiModal';
import useModeModeList from '@/renderer/hooks/agent/useModeModeList';
import type { DeepLinkAddProviderDetail } from '@/renderer/hooks/system/useDeepLink';
import {
  MODEL_PLATFORMS,
  getPlatformByValue,
  type PlatformConfig,
} from '@/renderer/utils/model/modelPlatforms';
import ModalHOC from '@/renderer/utils/ui/ModalHOC';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { AutoComplete, Form, Input, Select } from '@arco-design/web-react';
import { LinkCloud } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import ModelDefinitionEditor, { type ModelCatalogSuggestion } from './ModelDefinitionEditor';
import {
  capabilityInputsFromDefinition,
  emptyCapabilityDraft,
  normalizeModelId,
  validateModelDefinition,
  type ModelDefinitionDraft,
  type ProviderConnectionInput,
} from './providerModelAdvanced';
import {
  buildBedrockConfig,
  buildProviderCredentials,
  type BedrockAuthMethod,
} from './providerCredentialsForm';
import useModelProtocolManifests from './useModelProtocolManifests';

const EMPTY_DEFINITION: ModelDefinitionDraft = { model: '', capabilities: [] };

const ProviderLogo: React.FC<{ logo: string | null; name: string; size?: number }> = ({
  logo,
  name,
  size = 20,
}) =>
  logo ? (
    <img src={logo} alt={name} className='object-contain shrink-0' style={{ width: size, height: size }} />
  ) : (
    <LinkCloud theme='outline' size={size} className='text-t-secondary flex shrink-0' />
  );

const renderPlatformOption = (platform: PlatformConfig, t: (key: string) => string) => {
  const displayName = platform.i18nKey ? t(platform.i18nKey) : platform.name;
  return (
    <div className='flex items-center gap-8px'>
      <ProviderLogo logo={platform.logo} name={displayName} size={18} />
      <span>{displayName}</span>
    </div>
  );
};

const AddPlatformModal = ModalHOC<{
  onSubmit: (provider: IProvider) => void;
  deepLinkData?: DeepLinkAddProviderDetail;
}>(({ modalProps, onSubmit, modalCtrl, deepLinkData }) => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage();
  const [form] = Form.useForm();
  const [definition, setDefinition] = useState<ModelDefinitionDraft>(EMPTY_DEFINITION);
  const [pendingConnections, setPendingConnections] = useState<ProviderConnectionInput[]>([]);
  const [baseUrlDirty, setBaseUrlDirty] = useState(false);
  const [authSchemeDirty, setAuthSchemeDirty] = useState(false);
  const [saving, setSaving] = useState(false);

  const preset = Form.useWatch('platform', form) as string | undefined;
  const baseUrl = (Form.useWatch('base_url', form) as string | undefined) ?? '';
  const apiKey = (Form.useWatch('api_key', form) as string | undefined) ?? '';
  const authScheme = (Form.useWatch('auth_scheme', form) as string | undefined) ?? '';
  const bedrockAuthMethod = Form.useWatch('bedrockAuthMethod', form);
  const bedrockRegion = Form.useWatch('bedrockRegion', form);
  const bedrockAccessKeyId = Form.useWatch('bedrockAccessKeyId', form);
  const bedrockSecretAccessKey = Form.useWatch('bedrockSecretAccessKey', form);
  const bedrockSessionToken = Form.useWatch('bedrockSessionToken', form);
  const bedrockProfile = Form.useWatch('bedrockProfile', form);
  const selectedPlatform = useMemo(() => getPlatformByValue(preset ?? ''), [preset]);
  const tasks = useMemo(
    () => definition.capabilities.map((capability) => capability.task),
    [definition.capabilities]
  );
  const manifestState = useModelProtocolManifests(preset, tasks, 'chat');
  const providerManifest =
    manifestState.loadingTasks.length > 0
      ? undefined
      : (tasks.length > 0 ? manifestState.manifests[tasks[0]] : undefined) ??
        manifestState.manifests.chat;
  const runtimePlatform = providerManifest?.platform ?? selectedPlatform?.platform ?? 'custom';
  const isBedrock = runtimePlatform === 'bedrock';
  const bedrockConfig = useMemo(() => {
    if (!isBedrock || !bedrockAuthMethod || !bedrockRegion) return undefined;
    return buildBedrockConfig(
      bedrockAuthMethod as BedrockAuthMethod,
      String(bedrockRegion),
      String(bedrockProfile ?? '')
    );
  }, [
    bedrockAuthMethod,
    bedrockProfile,
    bedrockRegion,
    isBedrock,
  ]);
  const credentialsResult = useMemo(
    () =>
      buildProviderCredentials({
        isBedrock,
        mode: 'create',
        hasStoredCredentials: false,
        apiKeysText: apiKey,
        bedrockAuthMethod: bedrockAuthMethod as BedrockAuthMethod | undefined,
        accessKeyId: String(bedrockAccessKeyId ?? ''),
        secretAccessKey: String(bedrockSecretAccessKey ?? ''),
        sessionToken: String(bedrockSessionToken ?? ''),
      }),
    [
      apiKey,
      bedrockAccessKeyId,
      bedrockAuthMethod,
      bedrockSecretAccessKey,
      bedrockSessionToken,
      isBedrock,
    ]
  );
  const modelListState = useModeModeList({
    platform: runtimePlatform,
    baseUrl,
    authScheme,
    credentials:
      credentialsResult.ok &&
      (!isBedrock ||
        (bedrockConfig &&
          (bedrockAuthMethod !== 'profile' || String(bedrockProfile ?? '').trim().length > 0)))
        ? credentialsResult.credentials
        : undefined,
    bedrockConfig,
    tryFix: true,
  });
  const catalogSuggestions = useMemo<ModelCatalogSuggestion[]>(
    () =>
      (modelListState.data?.models ?? []).map((model) => ({
        value: model.value,
        label: model.label,
        tasks: model.tasks,
        traits: model.traits,
      })),
    [modelListState.data?.models]
  );
  const validation = useMemo(
    () =>
      validateModelDefinition(
        definition,
        manifestState.manifests,
        baseUrl,
        [],
        manifestState.loadingTasks,
        pendingConnections.map((connection) => connection.role),
        authScheme,
        Object.fromEntries(
          pendingConnections.map((connection) => [connection.role, connection.auth_scheme])
        ),
        pendingConnections
      ),
    [
      baseUrl,
      authScheme,
      definition,
      manifestState.loadingTasks,
      manifestState.manifests,
      pendingConnections,
    ]
  );

  const providerDisplayName = (platform: PlatformConfig | undefined): string => {
    if (!platform || platform.value === 'custom' || platform.value === 'new-api') return '';
    return platform.i18nKey ? t(platform.i18nKey) : platform.name;
  };

  useEffect(() => {
    if (!modalProps.visible) return;
    form.resetFields();
    setDefinition({
      model: deepLinkData?.model?.trim() ?? '',
      capabilities: deepLinkData?.task ? [emptyCapabilityDraft(deepLinkData.task)] : [],
    });
    setPendingConnections([]);
    setBaseUrlDirty(Boolean(deepLinkData?.base_url));
    setAuthSchemeDirty(false);
    setSaving(false);
    const requestedPreset = deepLinkData?.platform;
    const matchedPreset = requestedPreset
      ? MODEL_PLATFORMS.find(
          (platform) => platform.value === requestedPreset || platform.platform === requestedPreset
        )?.value
      : undefined;
    const initialPreset = matchedPreset ?? (deepLinkData ? 'new-api' : 'gemini');
    const initialPlatform = getPlatformByValue(initialPreset);
    form.setFieldsValue({
      platform: initialPreset,
      name: deepLinkData?.name ?? providerDisplayName(initialPlatform),
      base_url: deepLinkData?.base_url ?? '',
      api_key: '',
      auth_scheme: '',
      bedrockAuthMethod: 'accessKey',
      bedrockRegion: 'us-east-1',
      bedrockAccessKeyId: '',
      bedrockSecretAccessKey: '',
      bedrockSessionToken: '',
      bedrockProfile: '',
    });
  }, [deepLinkData, form, modalProps.visible]);

  useEffect(() => {
    if (!modalProps.visible || !providerManifest) return;
    if (!baseUrlDirty && providerManifest.platform_default_base_url) {
      form.setFieldValue('base_url', providerManifest.platform_default_base_url);
    }
    if (!authSchemeDirty && providerManifest.default_auth_scheme) {
      form.setFieldValue('auth_scheme', providerManifest.default_auth_scheme);
    }
  }, [
    authSchemeDirty,
    baseUrlDirty,
    form,
    modalProps.visible,
    providerManifest,
  ]);

  const selectPreset = (nextPreset: string) => {
    const platform = getPlatformByValue(nextPreset);
    setDefinition(EMPTY_DEFINITION);
    setPendingConnections([]);
    setBaseUrlDirty(false);
    setAuthSchemeDirty(false);
    form.setFieldsValue({
      name: providerDisplayName(platform),
      base_url: '',
      auth_scheme: '',
      api_key: '',
      bedrockAuthMethod: 'accessKey',
      bedrockRegion: 'us-east-1',
      bedrockAccessKeyId: '',
      bedrockSecretAccessKey: '',
      bedrockSessionToken: '',
      bedrockProfile: '',
    });
  };

  const addPendingConnection = async (connection: ProviderConnectionInput) => {
    setPendingConnections((current) => [
      ...current.filter((candidate) => candidate.role !== connection.role),
      connection,
    ]);
  };

  const submit = async () => {
    if (!validation.valid) {
      message.warning(
        t('settings.completeCapabilityConfiguration', {
          defaultValue: '请完成每个已选模态的协议、地址和连接配置。',
        })
      );
      return;
    }
    const capabilities = capabilityInputsFromDefinition(definition);
    if (!capabilities) return;
    try {
      const values = await form.validate();
      const credentialBuild = buildProviderCredentials({
        isBedrock,
        mode: 'create',
        hasStoredCredentials: false,
        apiKeysText: values.api_key,
        bedrockAuthMethod: values.bedrockAuthMethod,
        accessKeyId: values.bedrockAccessKeyId,
        secretAccessKey: values.bedrockSecretAccessKey,
        sessionToken: values.bedrockSessionToken,
      });
      if (!credentialBuild.ok || credentialBuild.credentials === undefined) {
        if (isBedrock) {
          message.error(t('settings.bedrock.credentialsRequired'));
          return;
        }
        message.error(t('settings.apiKeyRequired', { defaultValue: '请输入至少一个非空 API Key。' }));
        return;
      }
      const providerId = parseProviderId(uuidv7());
      setSaving(true);
      const created = await ipcBridge.mode.createProvider.invoke({
        id: providerId,
        platform: runtimePlatform,
        name: String(values.name).trim(),
        base_url: isBedrock ? '' : String(values.base_url ?? '').trim(),
        credentials: credentialBuild.credentials,
        auth_scheme: String(values.auth_scheme).trim(),
        enabled: true,
        initial_model: {
          model: normalizeModelId(definition.model),
          enabled: true,
          capabilities,
        },
        ...(pendingConnections.length > 0 ? { connections: pendingConnections } : {}),
        ...(isBedrock && bedrockConfig ? { bedrock_config: bedrockConfig } : {}),
      });
      onSubmit(created);
      modalCtrl.close();
    } catch (error) {
      console.error('atomic provider graph create failed', error);
      message.error(t('settings.saveModelConfigFailed', { defaultValue: '供应商和模型配置保存失败' }));
    } finally {
      setSaving(false);
    }
  };

  const authSchemeOptions = providerManifest?.auth_schemes.map((item) => item.scheme) ?? [];

  return (
    <NomiModal
      visible={modalProps.visible}
      onCancel={modalCtrl.close}
      unmountOnExit
      header={{ title: t('settings.addModel'), showClose: true }}
      style={{ width: 820, maxWidth: '95vw', maxHeight: '94vh', borderRadius: 16 }}
      contentStyle={{
        background: 'var(--dialog-fill-0)',
        borderRadius: 16,
        padding: '20px 24px 16px',
        overflow: 'auto',
      }}
      onOk={() => void submit()}
      confirmLoading={saving}
      okButtonProps={{ disabled: !validation.valid }}
      okText={t('common.confirm')}
      cancelText={t('common.cancel')}
    >
      {messageContext}
      <div className='pt-4px pb-12px flex flex-col gap-18px'>
        <Form form={form} layout='vertical' className='[&_.arco-form-item]:mb-12px'>
          <Form.Item label={t('settings.modelPlatform')} field='platform' required rules={[{ required: true }]}>
            <Select
              showSearch
              onChange={selectPreset}
              filterOption={(input, option) => {
                const value = (option as React.ReactElement<{ value?: string }>)?.props?.value;
                return (
                  MODEL_PLATFORMS.find((platform) => platform.value === value)?.name
                    .toLowerCase()
                    .includes(input.toLowerCase()) ?? false
                );
              }}
              renderFormat={(option) => {
                const platform = getPlatformByValue(String((option as { value?: string }).value ?? ''));
                return platform ? renderPlatformOption(platform, t) : String((option as { value?: string }).value ?? '');
              }}
            >
              {MODEL_PLATFORMS.map((platform) => (
                <Select.Option key={platform.value} value={platform.value}>
                  {renderPlatformOption(platform, t)}
                </Select.Option>
              ))}
            </Select>
          </Form.Item>

          <Form.Item
            label={t('settings.modelProvider')}
            field='name'
            required
            rules={[{ required: true }]}
          >
            <Input placeholder={t('settings.modelProvider')} />
          </Form.Item>

          <Form.Item
            label={t('settings.apiEndpoint', 'API 请求地址')}
            field='base_url'
            hidden={isBedrock}
            required={!isBedrock}
            rules={[{ required: !isBedrock }]}
            extra={t('settings.providerBaseUrlVisibleHint', {
              defaultValue: '预设默认值始终显示并可修改；模态卡内可设置任务级覆盖。',
            })}
          >
            <Input onChange={() => setBaseUrlDirty(true)} />
          </Form.Item>

          <Form.Item
            label={t('settings.authScheme', { defaultValue: '鉴权方式（auth_scheme）' })}
            field='auth_scheme'
            required
            rules={[{ required: true }]}
            extra={t('settings.authSchemeManifestHint', {
              defaultValue: '由后端预设推荐，也可手填已注册的参数化格式。',
            })}
          >
            <AutoComplete
              data={authSchemeOptions.map((scheme) => ({ value: scheme, name: scheme }))}
              loading={manifestState.loadingTasks.includes('chat')}
              placeholder='bearer / token / header_key:x-api-key'
              onChange={() => setAuthSchemeDirty(true)}
              triggerProps={{ getPopupContainer: () => document.body }}
            />
          </Form.Item>

          <Form.Item
            hidden={isBedrock}
            label={t('settings.apiKey')}
            field='api_key'
            required={!isBedrock}
            rules={[{ required: !isBedrock }]}
            extra={t('settings.multiApiKeyTip')}
          >
            <Input.TextArea rows={3} placeholder={t('settings.apiKeyPlaceholder')} />
          </Form.Item>

          <Form.Item
            hidden={!isBedrock}
            label={t('settings.bedrock.authMethod')}
            field='bedrockAuthMethod'
            required={isBedrock}
            rules={[{ required: isBedrock }]}
          >
            <Select>
              <Select.Option value='accessKey'>{t('settings.bedrock.authMethodAccessKey')}</Select.Option>
              <Select.Option value='profile'>{t('settings.bedrock.authMethodProfile')}</Select.Option>
              <Select.Option value='defaultChain'>{t('settings.bedrock.authMethodDefaultChain')}</Select.Option>
            </Select>
          </Form.Item>

          <Form.Item
            hidden={!isBedrock}
            label={t('settings.bedrock.region')}
            field='bedrockRegion'
            required={isBedrock}
            rules={[{ required: isBedrock }]}
          >
            <Input placeholder='us-east-1' />
          </Form.Item>

          <Form.Item
            hidden={!isBedrock || bedrockAuthMethod !== 'accessKey'}
            label={t('settings.bedrock.accessKeyId')}
            field='bedrockAccessKeyId'
            required={isBedrock && bedrockAuthMethod === 'accessKey'}
            rules={[{ required: isBedrock && bedrockAuthMethod === 'accessKey' }]}
          >
            <Input.Password placeholder='AKIA...' visibilityToggle />
          </Form.Item>

          <Form.Item
            hidden={!isBedrock || bedrockAuthMethod !== 'accessKey'}
            label={t('settings.bedrock.secretAccessKey')}
            field='bedrockSecretAccessKey'
            required={isBedrock && bedrockAuthMethod === 'accessKey'}
            rules={[{ required: isBedrock && bedrockAuthMethod === 'accessKey' }]}
          >
            <Input.Password visibilityToggle />
          </Form.Item>

          <Form.Item
            hidden={!isBedrock || bedrockAuthMethod !== 'accessKey'}
            label={t('settings.bedrock.sessionToken')}
            field='bedrockSessionToken'
            extra={t('settings.bedrock.sessionTokenHint')}
          >
            <Input.Password visibilityToggle />
          </Form.Item>

          <Form.Item
            hidden={!isBedrock || bedrockAuthMethod !== 'profile'}
            label={t('settings.bedrock.profile')}
            field='bedrockProfile'
            required={isBedrock && bedrockAuthMethod === 'profile'}
            rules={[{ required: isBedrock && bedrockAuthMethod === 'profile' }]}
            extra={t('settings.bedrock.profileHint')}
          >
            <Input placeholder='default' />
          </Form.Item>

          {isBedrock && bedrockAuthMethod === 'defaultChain' ? (
            <div className='text-12px text-t-secondary mb-12px'>
              {t('settings.bedrock.defaultChainHint')}
            </div>
          ) : null}
        </Form>

        <ModelDefinitionEditor
          value={definition}
          onChange={setDefinition}
          providerBaseUrl={baseUrl}
          providerAuthScheme={authScheme}
          manifests={manifestState.manifests}
          manifestLoadingTasks={manifestState.loadingTasks}
          manifestErrorTasks={manifestState.errorTasks}
          validationErrors={validation.errors}
          catalogSuggestions={catalogSuggestions}
          catalogLoading={modelListState.isLoading}
          catalogError={
            modelListState.error instanceof Error
              ? modelListState.error.message
              : modelListState.error
                ? String(modelListState.error)
                : undefined
          }
          onRefreshCatalog={() => void modelListState.mutate()}
          connections={pendingConnections}
          onCreateConnection={addPendingConnection}
        />
      </div>
    </NomiModal>
  );
});

export default AddPlatformModal;
