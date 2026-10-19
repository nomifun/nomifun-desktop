import { Alert, Form, Input, Modal, Select } from '@arco-design/web-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { McpOAuthClientConfig } from '@/common/types/mcpAuth';

/** Client secrets exist only in this mounted form and the write-only request. */
export default function McpOAuthClientModal({ serverName, onCancel, onSubmit }: {
  serverName: string;
  onCancel: () => void;
  onSubmit: (client: McpOAuthClientConfig) => void;
}) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<McpOAuthClientConfig['mode']>('auto');
  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
  const [authMethod, setAuthMethod] = useState('none');
  const [redirectUri, setRedirectUri] = useState('');
  const [error, setError] = useState('');

  const submit = () => {
    if (mode === 'auto') { onSubmit({ mode }); return; }
    if (!clientId.trim()) { setError(t('settings.mcpOAuthClientIdRequired')); return; }
    if (mode === 'cimd') {
      try {
        const url = new URL(clientId);
        if (url.protocol !== 'https:' || url.username || url.password || url.hash) throw new Error();
      } catch { setError(t('settings.mcpOAuthCimdHttpsRequired')); return; }
      onSubmit({ mode, client_id: clientId.trim() });
      return;
    }
    if (authMethod !== 'none' && !clientSecret) { setError(t('settings.mcpOAuthClientSecretRequired')); return; }
    onSubmit({ mode, client_id: clientId.trim(), client_secret: authMethod === 'none' ? undefined : clientSecret || undefined,
      token_endpoint_auth_method: authMethod, redirect_uri: redirectUri.trim() || undefined });
  };

  return <Modal title={`${serverName} · ${t('settings.mcpOAuthConfigureClient')}`} visible onCancel={onCancel} onOk={submit}
    okText={t('settings.mcpLogin')} cancelText={t('common.cancel')} style={{ width: 520 }}>
    <div className='mb-16px text-13px text-t-secondary'>{t('settings.mcpOAuthClientHint')}</div>
    <Form layout='vertical'>
      <Form.Item label={t('settings.mcpOAuthClientMode')}>
        <Select value={mode} onChange={(value) => { setMode(value); setError(''); }} options={[
          { value: 'auto', label: t('settings.mcpOAuthClientAuto') },
          { value: 'pre_registered', label: t('settings.mcpOAuthClientPreRegistered') },
          { value: 'cimd', label: t('settings.mcpOAuthClientCimd') },
        ]} />
      </Form.Item>
      {mode !== 'auto' && <Form.Item label={t(mode === 'cimd' ? 'settings.mcpOAuthCimdUrl' : 'settings.mcpOAuthClientId')} required>
        <Input value={clientId} onChange={setClientId} autoComplete='off' />
      </Form.Item>}
      {mode === 'pre_registered' && <>
        <Form.Item label={t('settings.mcpOAuthTokenAuthMethod')}>
          <Select value={authMethod} onChange={(value) => { setAuthMethod(value); if (value === 'none') setClientSecret(''); }} options={[
            { value: 'none', label: t('settings.mcpOAuthPublicClient') },
            { value: 'client_secret_basic', label: 'client_secret_basic' },
            { value: 'client_secret_post', label: 'client_secret_post' },
          ]} />
        </Form.Item>
        {authMethod !== 'none' && <Form.Item label={t('settings.mcpOAuthClientSecret')} required>
          <Input.Password value={clientSecret} onChange={setClientSecret} autoComplete='new-password' visibilityToggle />
        </Form.Item>}
        <Form.Item label={t('settings.mcpOAuthRedirectUri')} extra={t('settings.mcpOAuthRedirectHint')}>
          <Input value={redirectUri} onChange={setRedirectUri} autoComplete='off' />
        </Form.Item>
      </>}
    </Form>
    {error && <Alert type='error' content={error} />}
  </Modal>;
}
