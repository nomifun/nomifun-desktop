/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IProvider } from '@/common/config/storage';
import type { ModelGatewayCatalogResponse, ModelGatewayMetaResponse } from '@/common/types/provider/modelGateway';
import { Alert, Button, Checkbox, Input, Tag } from '@arco-design/web-react';
import React, { useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { gatewayAddress, gatewayExternalUrl } from './modelGatewayForm';

export const GatewayOperator: React.FC<{ meta: ModelGatewayMetaResponse }> = ({ meta }) => {
  const { t } = useTranslation();
  const links = [
    ['homepage_url', 'homepage'], ['console_url', 'console'], ['purchase_url', 'purchase'],
    ['terms_url', 'terms'], ['privacy_url', 'privacy'],
  ] as const;
  return (
    <div className='p-12px rd-8px border border-solid border-[var(--color-border-2)] flex flex-col gap-8px' data-gateway-operator>
      <strong className='text-14px text-t-primary'>{meta.operator.name}</strong>
      <div className='text-12px text-t-secondary'>{t('settings.modelGateway.independentOperator')}</div>
      <div className='flex flex-wrap gap-8px'>
        {links.map(([field, key]) => {
          const url = gatewayExternalUrl(meta.operator[field]);
          return url ? <Button key={field} size='mini' type='text' onClick={() => void ipcBridge.shell.openExternal.invoke(url)}>{t(`settings.modelGateway.links.${key}`)}</Button> : null;
        })}
      </div>
    </div>
  );
};

/** Gateway onboarding has its own control-plane flow; the generic model editor is unchanged. */
const ModelGatewaySetup: React.FC<{
  initialBaseUrl?: string;
  initialName?: string;
  onCreated(provider: IProvider): void;
  onCancel(): void;
}> = ({ initialBaseUrl = '', initialName = '', onCreated, onCancel }) => {
  const { t } = useTranslation();
  const [baseUrl, setBaseUrl] = useState(initialBaseUrl);
  const [name, setName] = useState(initialName);
  const [apiKey, setApiKey] = useState('');
  const [meta, setMeta] = useState<ModelGatewayMetaResponse>();
  const [catalog, setCatalog] = useState<ModelGatewayCatalogResponse>();
  const [selected, setSelected] = useState<string[]>([]);
  const [search, setSearch] = useState('');
  const [busy, setBusy] = useState<'meta' | 'catalog' | 'create'>();
  const [error, setError] = useState('');
  const revision = useRef(0);
  const address = gatewayAddress(baseUrl);
  const models = useMemo(() => catalog?.models.filter((model) =>
    `${model.id} ${model.display_name} ${model.vendor}`.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase())
  ) ?? [], [catalog, search]);

  const changeAddress = (value: string) => {
    revision.current += 1;
    setBaseUrl(value); setMeta(undefined); setCatalog(undefined); setSelected([]); setApiKey(''); setError(''); setBusy(undefined);
  };
  const changeKey = (value: string) => {
    revision.current += 1;
    setApiKey(value); setCatalog(undefined); setSelected([]); setError(''); setBusy(undefined);
  };
  const loadMeta = async () => {
    if (!address || busy) return;
    const current = ++revision.current;
    setBusy('meta'); setError('');
    try {
      const next = await ipcBridge.modelGateway.meta.invoke({ base_url: address.root });
      if (revision.current !== current) return;
      setMeta(next);
      if (!name.trim()) setName(next.operator.name);
    } catch { if (revision.current === current) setError(t('settings.modelGateway.metaFailed')); }
    finally { if (revision.current === current) setBusy(undefined); }
  };
  const loadCatalog = async () => {
    if (!address || !meta || !apiKey.trim() || busy) return;
    const current = ++revision.current;
    setBusy('catalog'); setError('');
    try {
      const next = await ipcBridge.modelGateway.catalog.invoke({ base_url: address.root, api_key: apiKey.trim() });
      if (revision.current !== current) return;
      setCatalog(next); setSelected(next.models.map((model) => model.id));
    } catch { if (revision.current === current) setError(t('settings.modelGateway.catalogFailed')); }
    finally { if (revision.current === current) setBusy(undefined); }
  };
  const save = async () => {
    if (!address || !meta || !catalog || !apiKey.trim() || !name.trim() || selected.length === 0 || busy) return;
    setBusy('create'); setError('');
    try {
      const provider = await ipcBridge.modelGateway.create.invoke({ base_url: address.root, api_key: apiKey.trim(), name: name.trim(), models: selected });
      setApiKey(''); onCreated(provider);
    } catch { setError(t('settings.modelGateway.saveFailed')); }
    finally { setBusy(undefined); }
  };
  return (
    <div className='flex flex-col gap-12px' data-model-gateway-setup>
      <div className='text-12px text-t-secondary'>{t('settings.modelGateway.setupDescription')}</div>
      <label className='flex flex-col gap-6px'>
        <span>{t('settings.modelGateway.address')}</span>
        <Input value={baseUrl} onChange={changeAddress} disabled={busy === 'create'} placeholder='https://models.example.com' aria-label={t('settings.modelGateway.address')} />
      </label>
      {baseUrl.trim() && !address && <Alert type='error' content={t('settings.modelGateway.invalidAddress')} />}
      {address?.insecure && <Alert type='warning' content={t('settings.modelGateway.insecureAddress')} />}
      <Button onClick={() => void loadMeta()} disabled={!address || !!busy} loading={busy === 'meta'}>{t('settings.modelGateway.checkOperator')}</Button>
      {meta && <>
        <GatewayOperator meta={meta} />
        <label className='flex flex-col gap-6px'><span>{t('settings.modelProvider')}</span><Input value={name} onChange={setName} disabled={busy === 'create'} aria-label={t('settings.modelProvider')} /></label>
        <label className='flex flex-col gap-6px'>
          <span>{t('settings.apiKey')}</span>
          <Input.Password value={apiKey} onChange={changeKey} disabled={busy === 'create'} aria-label={t('settings.apiKey')} autoComplete='off' />
          <span className='text-12px text-t-secondary'>{t('settings.modelGateway.keyDestination', { domain: address?.domain ?? '' })}</span>
        </label>
        <Button onClick={() => void loadCatalog()} disabled={!apiKey.trim() || !!busy} loading={busy === 'catalog'}>{t('settings.modelGateway.loadCatalog')}</Button>
      </>}
      {catalog && <>
        <Input.Search value={search} onChange={setSearch} placeholder={t('settings.modelGateway.searchModels')} aria-label={t('settings.modelGateway.searchModels')} />
        <Checkbox checked={selected.length === catalog.models.length && catalog.models.length > 0} indeterminate={selected.length > 0 && selected.length < catalog.models.length} disabled={busy === 'create'} onChange={(checked) => setSelected(checked ? catalog.models.map((model) => model.id) : [])}>{t('settings.modelGateway.selectAll', { selected: selected.length, total: catalog.models.length })}</Checkbox>
        {catalog.models.length === 0 && <Alert content={t('settings.modelGateway.emptyCatalog')} />}
        <div className='max-h-240px overflow-auto flex flex-col gap-8px' data-gateway-catalog>
          {models.map((model) => <div key={model.id} className='p-8px border border-solid border-[var(--color-border-2)] rd-6px'>
            <Checkbox checked={selected.includes(model.id)} disabled={busy === 'create'} onChange={(checked) => setSelected((current) => checked ? [...current, model.id] : current.filter((id) => id !== model.id))}>
              <span className='font-500'>{model.display_name}</span><span className='text-12px text-t-secondary ml-8px'>{model.id} · {model.vendor}</span>
            </Checkbox>
            <div className='flex flex-wrap gap-4px mt-4px'>{model.tasks.map((task) => <Tag key={task} size='small'>{t(`settings.modelTask.${task}`)}</Tag>)}<Tag size='small'>{t(`settings.modelGateway.status.${model.status}`, { defaultValue: model.status })}</Tag>{model.included_in_plan && <Tag size='small' color='green'>{t('settings.modelGateway.includedInPlan')}</Tag>}</div>
          </div>)}
        </div>
      </>}
      {error && <Alert type='error' content={error} />}
      <div className='flex justify-end gap-8px mt-4px'>
        <Button onClick={onCancel} disabled={busy === 'create'}>{t('common.cancel')}</Button>
        <Button type='primary' onClick={() => void save()} loading={busy === 'create'} disabled={!catalog || !name.trim() || selected.length === 0 || !!busy}>{t('settings.modelGateway.addSelected')}</Button>
      </div>
    </div>
  );
};
export default ModelGatewaySetup;
