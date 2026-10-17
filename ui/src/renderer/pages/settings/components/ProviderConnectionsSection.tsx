/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Input, Popconfirm, Select, Tag, Tooltip } from '@arco-design/web-react';
import { DeleteFour, Down, LinkCloud, Plus, Right, Write } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { IProvider } from '@/common/config/storage';
import type { ProviderConnectionResponse } from '@/common/types/provider/providerConnection';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import ModelConfigurationPanel from '@/renderer/pages/modelHub/ModelConfigurationPanel';
import {
  AUTH_SCHEME_PRESETS,
  buildConnectionCredentials,
  credentialsKindForScheme,
  isValidConnectionRole,
  isVolcArkPlatform,
  type ConnectionCredentialsDraft,
} from './providerConnectionForm';
import { useProviderConnections } from './useProviderConnections';

const CUSTOM_SCHEME = '__custom__';

type FormState = {
  editing?: ProviderConnectionResponse;
  prefillRole?: string;
  prefillScheme?: string;
};

const emptyCredentialsDraft: ConnectionCredentialsDraft = {
  apiKeysText: '',
  appKey: '',
  accessKey: '',
  resourceId: '',
  rawJson: '',
};

/**
 * Add/edit page for one per-role connection profile. Credentials are
 * write-only: in edit mode an empty credentials form keeps the stored ones.
 */
const ConnectionForm: React.FC<{
  provider: IProvider;
  state: FormState;
  onClose: () => void;
  onSaved: () => void;
}> = ({ provider, state, onClose, onSaved }) => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage();
  const { editing, prefillRole, prefillScheme } = state;
  const isEdit = Boolean(editing);

  const initialScheme = editing?.auth_scheme ?? prefillScheme ?? 'bearer';
  const schemeIsPreset = (AUTH_SCHEME_PRESETS as readonly string[]).includes(initialScheme);

  const [role, setRole] = useState(editing?.role ?? prefillRole ?? '');
  const [label, setLabel] = useState(editing?.label ?? '');
  const [baseUrl, setBaseUrl] = useState(editing?.base_url ?? '');
  const [schemeSelect, setSchemeSelect] = useState(schemeIsPreset ? initialScheme : CUSTOM_SCHEME);
  const [customScheme, setCustomScheme] = useState(schemeIsPreset ? '' : initialScheme);
  const [creds, setCreds] = useState<ConnectionCredentialsDraft>(emptyCredentialsDraft);
  const [saving, setSaving] = useState(false);

  const scheme = schemeSelect === CUSTOM_SCHEME ? customScheme.trim() : schemeSelect;
  const credentialsKind = credentialsKindForScheme(scheme || 'bearer');

  const schemeOptions = [
    ...AUTH_SCHEME_PRESETS.map((value) => ({ label: value, value })),
    { label: t('settings.connections.authSchemeCustom', { defaultValue: '自定义' }), value: CUSTOM_SCHEME },
  ];

  const handleSave = async () => {
    if (saving) return;
    const nextRole = role.trim();
    if (!isValidConnectionRole(nextRole)) {
      message.error(
        t('settings.connections.roleInvalid', {
          defaultValue: 'role 不合法：需匹配 ^[a-z][a-z0-9_-]{0,31}$ 且不能为 default',
        })
      );
      return;
    }
    if (!baseUrl.trim()) {
      message.error(t('settings.connections.baseUrlRequired', { defaultValue: '请填写请求地址' }));
      return;
    }
    if (!scheme) {
      message.error(t('settings.connections.authSchemeRequired', { defaultValue: '请填写鉴权方式' }));
      return;
    }
    const built = buildConnectionCredentials(scheme, creds);
    if (!built.ok) {
      const errorKey =
        built.error === 'volc_incomplete'
          ? 'settings.connections.volcIncomplete'
          : 'settings.connections.invalidCredentialsJson';
      message.error(t(errorKey));
      return;
    }
    if (built.credentials === undefined && (!isEdit || editing?.has_credentials !== true)) {
      message.error(t('settings.connections.credentialsRequired'));
      return;
    }
    setSaving(true);
    try {
      await ipcBridge.providerConnection.save.invoke({
        provider_id: provider.id,
        connection: {
          role: nextRole,
          label: label.trim() || undefined,
          base_url: baseUrl.trim(),
          auth_scheme: scheme,
          ...(built.credentials !== undefined ? { credentials: built.credentials } : {}),
        },
      });
      message.success(t('settings.connections.saved', { defaultValue: '连接档案已保存' }));
      onSaved();
      onClose();
    } catch (error) {
      console.error('Failed to save provider connection:', error);
      message.error(t('settings.saveModelConfigFailed'));
    } finally {
      setSaving(false);
    }
  };

  return (
    <ModelConfigurationPanel
      visible
      title={
        isEdit
          ? t('settings.connections.edit', { defaultValue: '编辑连接档案' })
          : t('settings.connections.add', { defaultValue: '添加连接档案' })
      }
      subtitle={provider.name}
      onBack={onClose}
      busy={saving}
      footer={
        <div className='flex justify-end gap-8px'>
          <Button onClick={onClose} disabled={saving}>
            {t('common.cancel', { defaultValue: '取消' })}
          </Button>
          <Button type='primary' onClick={() => void handleSave()} loading={saving} disabled={saving}>
            {t('common.save', { defaultValue: '保存' })}
          </Button>
        </div>
      }
    >
      {messageContext}
      <div className='flex flex-col gap-14px'>
        <div className='space-y-6px'>
          <div className='text-13px font-500 text-t-secondary'>
            {t('settings.connections.role', { defaultValue: '角色（role）' })}
          </div>
          <Input autoCapitalize='none' autoCorrect='off' spellCheck={false} value={role} onChange={setRole} disabled={isEdit} placeholder='voice' />
          {!isEdit && (
            <div className='text-11px text-t-secondary leading-4'>
              {t('settings.connections.roleHint', {
                defaultValue: '小写字母开头，可含数字、下划线、连字符，最长 32 位；default 为保留角色',
              })}
            </div>
          )}
        </div>

        <div className='space-y-6px'>
          <div className='text-13px font-500 text-t-secondary'>
            {t('settings.connections.label', { defaultValue: '名称' })}
          </div>
          <Input autoCapitalize='none' autoCorrect='off' spellCheck={false} value={label} onChange={setLabel} />
        </div>

        <div className='space-y-6px'>
          <div className='text-13px font-500 text-t-secondary'>
            {t('settings.connections.baseUrl', { defaultValue: '请求地址（base_url）' })}
          </div>
          <Input autoCapitalize='none' autoCorrect='off' spellCheck={false} value={baseUrl} onChange={setBaseUrl} placeholder='https://openspeech.bytedance.com' />
        </div>

        <div className='space-y-6px'>
          <div className='text-13px font-500 text-t-secondary'>
            {t('settings.connections.authScheme', { defaultValue: '鉴权方式（auth_scheme）' })}
          </div>
          <Select value={schemeSelect} onChange={setSchemeSelect} options={schemeOptions} />
          {schemeSelect === CUSTOM_SCHEME && (
            <Input
              autoCapitalize='none'
              autoCorrect='off'
              spellCheck={false}
              value={customScheme}
              onChange={setCustomScheme}
              placeholder={t('settings.connections.authSchemeCustomPlaceholder', {
                defaultValue: '如 query_key:key',
              })}
            />
          )}
        </div>

        <div className='space-y-6px'>
          <div className='text-13px font-500 text-t-secondary'>
            {t('settings.connections.credentials', { defaultValue: '凭证' })}
          </div>
          {credentialsKind === 'api_keys' && (
            <Input.TextArea
              autoCapitalize='none'
              autoCorrect='off'
              spellCheck={false}
              value={creds.apiKeysText}
              onChange={(apiKeysText) => setCreds((prev) => ({ ...prev, apiKeysText }))}
              placeholder={t('settings.connections.apiKeys', {
                defaultValue: 'API Key（多个用逗号或换行分隔）',
              })}
              autoSize={{ minRows: 2, maxRows: 5 }}
            />
          )}
          {credentialsKind === 'volc_voice' && (
            <div className='flex flex-col gap-8px'>
              <Input
                autoCapitalize='none'
                autoCorrect='off'
                spellCheck={false}
                value={creds.appKey}
                onChange={(appKey) => setCreds((prev) => ({ ...prev, appKey }))}
                placeholder={t('settings.connections.volcAppKey', { defaultValue: 'App Key' })}
              />
              <Input
                autoCapitalize='none'
                autoCorrect='off'
                spellCheck={false}
                value={creds.accessKey}
                onChange={(accessKey) => setCreds((prev) => ({ ...prev, accessKey }))}
                placeholder={t('settings.connections.volcAccessKey', { defaultValue: 'Access Key' })}
              />
              <Input
                autoCapitalize='none'
                autoCorrect='off'
                spellCheck={false}
                value={creds.resourceId}
                onChange={(resourceId) => setCreds((prev) => ({ ...prev, resourceId }))}
                placeholder={t('settings.connections.volcResourceId', { defaultValue: 'Resource ID' })}
              />
            </div>
          )}
          {credentialsKind === 'custom' && (
            <Input.TextArea
              autoCapitalize='none'
              autoCorrect='off'
              spellCheck={false}
              value={creds.rawJson}
              onChange={(rawJson) => setCreds((prev) => ({ ...prev, rawJson }))}
              placeholder={t('settings.connections.rawCredentials', { defaultValue: '凭证 JSON' })}
              autoSize={{ minRows: 3, maxRows: 8 }}
            />
          )}
          {isEdit && (
            <div className='text-11px text-t-secondary leading-4'>
              {t('settings.connections.keepCredentialsHint', {
                defaultValue: '留空保持现有凭证不变',
              })}
            </div>
          )}
        </div>

      </div>
    </ModelConfigurationPanel>
  );
};

/**
 * Collapsed "连接档案 Connections" section inside a provider card: lists the
 * provider's non-default per-role connection profiles with add/edit/delete.
 * Ark/Volcengine providers get a hint banner pointing at the `voice` role
 * needed for speech recognition/synthesis.
 */
const ProviderConnectionsSection: React.FC<{ provider: IProvider }> = ({ provider }) => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage();
  const [expanded, setExpanded] = useState(false);
  const [form, setForm] = useState<FormState | null>(null);
  const { connections, isLoading, mutate } = useProviderConnections(provider.id, expanded);

  // Providers freshly created from the volc/ark family most likely need the
  // voice connection — surface the section so the hint banner is visible.
  const showVoiceHint = isVolcArkPlatform(provider.platform);
  useEffect(() => {
    if (showVoiceHint) setExpanded(true);
  }, [showVoiceHint]);

  const removeConnection = async (role: string) => {
    try {
      await ipcBridge.providerConnection.remove.invoke({ provider_id: provider.id, role });
      message.success(t('settings.connections.deleted', { defaultValue: '连接档案已删除' }));
      void mutate();
    } catch (error) {
      console.error('Failed to delete provider connection:', error);
      message.error(t('settings.saveModelConfigFailed'));
    }
  };

  return (
    <div className='px-8px pt-8px pb-4px'>
      {messageContext}
      <div className='flex items-center justify-between gap-8px'>
        <button
          type='button'
          className='flex items-center gap-6px bg-transparent border-0 p-0 cursor-pointer text-12px text-t-secondary hover:text-t-primary transition-colors'
          onClick={() => setExpanded((prev) => !prev)}
        >
          {expanded ? <Down theme='outline' size='12' /> : <Right theme='outline' size='12' />}
          <LinkCloud theme='outline' size='12' />
          <span>
            {t('settings.connections.title', { defaultValue: '连接档案' })}
            {expanded && !isLoading ? `（${connections.length}）` : ''}
          </span>
        </button>
        {expanded && (
          <Button
            size='mini'
            className='model-provider-action-btn !h-24px !min-w-24px shrink-0 px-6px text-t-secondary hover:text-t-primary'
            icon={<Plus size='12' />}
            onClick={() => setForm({})}
          >
            {t('settings.connections.add', { defaultValue: '添加连接档案' })}
          </Button>
        )}
      </div>

      {expanded && (
        <div className='mt-8px flex flex-col gap-6px'>
          {showVoiceHint && (
            <div
              className='rd-8px px-10px py-8px border border-solid flex items-center justify-between gap-8px'
              style={{
                borderColor: 'rgba(var(--primary-6),0.24)',
                backgroundColor: 'rgba(var(--primary-6),0.06)',
              }}
            >
              <span className='text-12px leading-5 text-t-primary min-w-0'>
                {t('settings.connections.voiceHint', {
                  defaultValue: '语音识别/合成需要独立的语音连接档案（role: voice）',
                })}
              </span>
              <Button
                size='mini'
                type='secondary'
                className='shrink-0'
                onClick={() => setForm({ prefillRole: 'voice', prefillScheme: 'volc_voice' })}
              >
                {t('settings.connections.voiceHintAction', { defaultValue: '配置语音连接' })}
              </Button>
            </div>
          )}

          {connections.length === 0 && !isLoading && (
            <div className='text-12px text-t-secondary leading-5 px-2px'>
              {t('settings.connections.empty', {
                defaultValue: '暂无独立连接档案（默认使用供应商自身的地址与密钥）',
              })}
            </div>
          )}

          {connections.map((connection) => (
            <div
              key={connection.connection_id}
              className='flex items-center justify-between gap-8px rd-8px px-8px py-6px bg-[var(--color-bg-2)]'
            >
              <div className='flex items-center gap-8px min-w-0 flex-1'>
                <Tag size='small' color='arcoblue' bordered className='shrink-0 select-none'>
                  {connection.role}
                </Tag>
                {connection.label && (
                  <span className='text-12px text-t-primary truncate min-w-0'>{connection.label}</span>
                )}
                <span className='text-12px text-t-secondary truncate min-w-0' title={connection.base_url}>
                  {connection.base_url}
                </span>
                <Tag size='small' bordered className='shrink-0 select-none'>
                  {connection.auth_scheme}
                </Tag>
                <Tag
                  size='small'
                  color={connection.has_credentials ? 'green' : 'gray'}
                  className='shrink-0 select-none'
                >
                  {connection.has_credentials
                    ? t('settings.connections.hasCredentials', { defaultValue: '已配置凭证' })
                    : t('settings.connections.noCredentials', { defaultValue: '未配置凭证' })}
                </Tag>
              </div>
              <div className='flex items-center gap-4px shrink-0'>
                <Tooltip content={t('settings.connections.edit', { defaultValue: '编辑连接档案' })}>
                  <Button
                    size='mini'
                    className='model-provider-action-btn !w-24px !h-24px !min-w-24px text-t-secondary hover:text-t-primary'
                    icon={<Write size='12' />}
                    onClick={() => setForm({ editing: connection })}
                  />
                </Tooltip>
                <Popconfirm
                  title={t('settings.connections.deleteConfirm', { defaultValue: '删除该连接档案？' })}
                  onOk={() => removeConnection(connection.role)}
                >
                  <Button
                    size='mini'
                    className='model-provider-action-btn !w-24px !h-24px !min-w-24px text-t-secondary hover:text-t-primary'
                    icon={<DeleteFour theme='outline' size='12' />}
                  />
                </Popconfirm>
              </div>
            </div>
          ))}
        </div>
      )}

      {form && (
        <ConnectionForm
          provider={provider}
          state={form}
          onClose={() => setForm(null)}
          onSaved={() => void mutate()}
        />
      )}
    </div>
  );
};

export default ProviderConnectionsSection;
