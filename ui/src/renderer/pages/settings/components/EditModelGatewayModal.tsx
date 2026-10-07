/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IProvider } from '@/common/config/storage';
import NomiModal from '@/renderer/components/base/NomiModal';
import ModalHOC from '@/renderer/utils/ui/ModalHOC';
import { Alert, Button, Input } from '@arco-design/web-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { gatewayAddress } from './modelGatewayForm';

const EditModelGatewayModal = ModalHOC<{ data?: IProvider; onChanged(): Promise<unknown> }>(({ data, modalProps, modalCtrl, onChanged }) => {
  const { t } = useTranslation();
  const [baseUrl, setBaseUrl] = useState('');
  const [name, setName] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState(false);
  useEffect(() => {
    if (!modalProps.visible || !data) return;
    setBaseUrl(gatewayAddress(data.base_url)?.root ?? data.base_url);
    setName(data.name); setApiKey(''); setError(false); setSaving(false);
  }, [data, modalProps.visible]);
  const address = gatewayAddress(baseUrl);
  const originalAddress = data && gatewayAddress(data.base_url);
  const destinationChanged = !!address && !!originalAddress && address.root !== originalAddress.root;
  const save = async () => {
    if (!data || !address || !name.trim() || saving || (destinationChanged && !apiKey.trim())) return;
    setSaving(true); setError(false);
    try {
      await ipcBridge.modelGateway.updateConnection.invoke({ provider_id: data.id, base_url: address.root, name: name.trim(), ...(apiKey.trim() ? { api_key: apiKey.trim() } : {}) });
      await onChanged(); setApiKey(''); modalCtrl.close();
    } catch { setError(true); }
    finally { setSaving(false); }
  };
  return <NomiModal visible={modalProps.visible} onCancel={modalCtrl.close} header={{ title: t('settings.modelGateway.edit'), showClose: !saving }} style={{ width: 620, maxWidth: '95vw' }} contentStyle={{ padding: '16px 24px' }} footer={<div className='flex justify-end gap-8px'><Button onClick={modalCtrl.close} disabled={saving}>{t('common.cancel')}</Button><Button type='primary' onClick={() => void save()} loading={saving} disabled={!address || !name.trim() || (destinationChanged && !apiKey.trim())}>{t('common.save')}</Button></div>} unmountOnExit>
    <div className='flex flex-col gap-12px'>
      <label className='flex flex-col gap-6px'><span>{t('settings.modelProvider')}</span><Input value={name} onChange={setName} disabled={saving} aria-label={t('settings.modelProvider')} /></label>
      <label className='flex flex-col gap-6px'><span>{t('settings.modelGateway.address')}</span><Input value={baseUrl} onChange={setBaseUrl} disabled={saving} aria-label={t('settings.modelGateway.address')} /></label>
      {!address && <Alert type='error' content={t('settings.modelGateway.invalidAddress')} />}
      {address?.insecure && <Alert type='warning' content={t('settings.modelGateway.insecureAddress')} />}
      <label className='flex flex-col gap-6px'><span>{t('settings.apiKey')}</span><Input.Password value={apiKey} onChange={setApiKey} disabled={saving} autoComplete='off' aria-label={t('settings.apiKey')} /><span className='text-12px text-t-secondary'>{t('settings.modelGateway.keyDestination', { domain: address?.domain ?? '' })}</span></label>
      <div className='text-12px text-t-secondary'>{t(destinationChanged ? 'settings.modelGateway.keyRequiredOnAddressChange' : 'settings.modelGateway.keepKey')}</div>
      <div className='text-12px text-t-secondary'>{t('settings.modelGateway.atomicConnections')}</div>
      {error && <Alert type='error' content={t('settings.modelGateway.saveFailed')} />}
    </div>
  </NomiModal>;
});
export default EditModelGatewayModal;
