/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider } from '@/common/config/storage';
import type { ProviderCredentials } from '@/common/types/provider/providerApi';
import NomiModal from '@/renderer/components/base/NomiModal';
import { getProviderLogo } from '@/renderer/utils/model/modelPlatforms';
import ModalHOC from '@/renderer/utils/ui/ModalHOC';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { AutoComplete, Form, Input, Select } from '@arco-design/web-react';
import { LinkCloud } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  buildBedrockConfig,
  buildProviderCredentials,
  type BedrockAuthMethod,
} from './providerCredentialsForm';
import useModelProtocolManifests from './useModelProtocolManifests';

const AWS_REGIONS = [
  'us-east-1',
  'us-west-2',
  'eu-west-1',
  'eu-central-1',
  'ap-southeast-1',
  'ap-northeast-1',
  'ap-southeast-2',
  'ca-central-1',
];

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

export type EditProviderPatch = Pick<IProvider, 'id' | 'name' | 'base_url' | 'auth_scheme'> & {
  /** Write-only. Omitted means keep the existing encrypted payload. */
  credentials?: ProviderCredentials;
  bedrock_config?: IProvider['bedrock_config'];
};

const EditModeModal = ModalHOC<{ data?: IProvider; onChange(data: EditProviderPatch): Promise<void> }>(
  ({ modalProps, modalCtrl, data, onChange }) => {
    const { t } = useTranslation();
    const [form] = Form.useForm();
    const [message, messageContext] = useArcoMessage();
    const [saving, setSaving] = useState(false);
    const bedrockAuthMethod = Form.useWatch('bedrockAuthMethod', form) as
      | BedrockAuthMethod
      | undefined;
    const isBedrock = data?.platform === 'bedrock';
    const credentialsCanBePreserved = data?.has_credentials === true;
    const accessKeysRequired =
      isBedrock && bedrockAuthMethod === 'accessKey' && !credentialsCanBePreserved;
    const manifestState = useModelProtocolManifests(data?.platform, [], 'chat', data?.base_url);
    const providerManifest = manifestState.manifests.chat;
    const authSchemeOptions = providerManifest?.auth_schemes.map((item) => item.scheme) ?? [];
    const providerLogo = useMemo(
      () => getProviderLogo({ name: data?.name, platform: data?.platform }),
      [data?.name, data?.platform]
    );

    useEffect(() => {
      if (!data || !modalProps.visible) return;
      form.resetFields();
      form.setFieldsValue({
        name: data.name,
        base_url: data.base_url,
        auth_scheme: data.auth_scheme,
        // Provider APIs never return secrets. Empty inputs mean preserve on update.
        api_key: '',
        bedrockAuthMethod: data.bedrock_config?.auth_method || 'accessKey',
        bedrockRegion: data.bedrock_config?.region || 'us-east-1',
        bedrockAccessKeyId: '',
        bedrockSecretAccessKey: '',
        bedrockSessionToken: '',
        bedrockProfile: data.bedrock_config?.profile || '',
      });
    }, [data, form, modalProps.visible]);

    useEffect(() => {
      if (
        modalProps.visible &&
        !form.getFieldValue('auth_scheme') &&
        providerManifest?.default_auth_scheme
      ) {
        form.setFieldValue('auth_scheme', providerManifest.default_auth_scheme);
      }
    }, [form, modalProps.visible, providerManifest?.default_auth_scheme]);

    const showCredentialError = (error: string) => {
      if (error === 'api_keys_required') {
        message.error(t('settings.apiKeyRequired'));
      } else if (error === 'bedrock_access_keys_incomplete') {
        message.error(t('settings.bedrock.accessKeysIncomplete'));
      } else {
        message.error(t('settings.bedrock.credentialsRequired'));
      }
    };

    const save = async () => {
      try {
        const values = await form.validate();
        if (!data) return;
        const credentialBuild = buildProviderCredentials({
          isBedrock,
          mode: 'update',
          hasStoredCredentials: data.has_credentials,
          apiKeysText: values.api_key,
          bedrockAuthMethod: values.bedrockAuthMethod,
          accessKeyId: values.bedrockAccessKeyId,
          secretAccessKey: values.bedrockSecretAccessKey,
          sessionToken: values.bedrockSessionToken,
        });
        if (!credentialBuild.ok) {
          showCredentialError(credentialBuild.error);
          return;
        }
        const patch: EditProviderPatch = {
          id: data.id,
          name: String(values.name).trim(),
          base_url: String(values.base_url ?? '').trim(),
          auth_scheme: String(values.auth_scheme).trim(),
          ...(credentialBuild.credentials === undefined
            ? {}
            : { credentials: credentialBuild.credentials }),
          ...(isBedrock
            ? {
                bedrock_config: buildBedrockConfig(
                  values.bedrockAuthMethod,
                  String(values.bedrockRegion),
                  values.bedrockProfile
                ),
              }
            : {}),
        };
        setSaving(true);
        await onChange(patch);
        modalCtrl.close();
      } catch {
        // Arco marks validation errors. The parent reports persistence failures.
      } finally {
        setSaving(false);
      }
    };

    const storedCredentialsHint = credentialsCanBePreserved ? (
      <div className='text-11px text-t-secondary mt-2'>
        {t('settings.connections.hasCredentials')} · {t('settings.connections.keepCredentialsHint')}
      </div>
    ) : undefined;

    return (
      <NomiModal
        visible={modalProps.visible}
        onCancel={modalCtrl.close}
        header={{ title: t('settings.editModel'), showClose: true }}
        style={{ minHeight: 400, maxHeight: '90vh', borderRadius: 16 }}
        contentStyle={{
          background: 'var(--dialog-fill-0)',
          borderRadius: 16,
          padding: '20px 24px 16px',
          overflow: 'auto',
        }}
        onOk={() => void save()}
        confirmLoading={modalProps.confirmLoading || saving}
        okText={t('common.save')}
        cancelText={t('common.cancel')}
      >
        {messageContext}
        <div className='py-20px'>
          <Form form={form} layout='vertical'>
            <Form.Item
              label={
                <div className='flex items-center gap-6px'>
                  <ProviderLogo logo={providerLogo} name={data?.name || ''} size={16} />
                  <span>{t('settings.modelProvider')}</span>
                </div>
              }
              field='name'
              required
              rules={[{ required: true }]}
            >
              <Input placeholder={t('settings.modelProvider')} />
            </Form.Item>

            <Form.Item
              label={t('settings.apiEndpoint')}
              field='base_url'
              required={!isBedrock}
              rules={[{ required: !isBedrock }]}
              extra={t('settings.providerBaseUrlVisibleHint')}
            >
              <Input />
            </Form.Item>

            <Form.Item
              label={t('settings.authScheme')}
              field='auth_scheme'
              required
              rules={[{ required: true }]}
              extra={t('settings.authSchemeManifestHint')}
            >
              <AutoComplete
                data={authSchemeOptions.map((scheme) => ({ value: scheme, name: scheme }))}
                loading={manifestState.loadingTasks.includes('chat')}
                placeholder='bearer / header_key:x-api-key'
                triggerProps={{ getPopupContainer: () => document.body }}
              />
            </Form.Item>

            <Form.Item
              hidden={isBedrock}
              label={t('settings.apiKey')}
              field='api_key'
              required={!isBedrock && !credentialsCanBePreserved}
              rules={[{ required: !isBedrock && !credentialsCanBePreserved }]}
              extra={
                storedCredentialsHint ?? (
                  <div className='text-11px text-t-secondary mt-2'>
                    {t('settings.multiApiKeyEditTip')}
                  </div>
                )
              }
            >
              <Input.TextArea rows={4} placeholder={t('settings.apiKeyPlaceholder')} />
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
              extra={t('settings.bedrock.regionHint')}
            >
              <AutoComplete data={AWS_REGIONS.map((region) => ({ value: region, name: region }))} />
            </Form.Item>

            <Form.Item
              hidden={!isBedrock || bedrockAuthMethod !== 'accessKey'}
              label={t('settings.bedrock.accessKeyId')}
              field='bedrockAccessKeyId'
              required={accessKeysRequired}
              rules={[{ required: accessKeysRequired }]}
              extra={storedCredentialsHint}
            >
              <Input.Password placeholder='AKIA...' visibilityToggle />
            </Form.Item>

            <Form.Item
              hidden={!isBedrock || bedrockAuthMethod !== 'accessKey'}
              label={t('settings.bedrock.secretAccessKey')}
              field='bedrockSecretAccessKey'
              required={accessKeysRequired}
              rules={[{ required: accessKeysRequired }]}
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
        </div>
      </NomiModal>
    );
  }
);

export default EditModeModal;
