/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IProvider } from '@/common/config/storage';
import type { ModelGatewayAccountResponse, ModelGatewayMetaResponse } from '@/common/types/provider/modelGateway';
import { Alert, Button } from '@arco-design/web-react';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { GatewayOperator } from './ModelGatewaySetup';
import { gatewayDate, gatewayExternalUrl, gatewayInteger, gatewayMoney } from './modelGatewayForm';

const ModelGatewayDetails: React.FC<{ provider: IProvider; onChanged(): Promise<unknown> }> = ({ provider, onChanged }) => {
  const { t, i18n } = useTranslation();
  const [syncNotice, setSyncNotice] = useState<{ type: 'success' | 'error'; text: string }>();
  const [meta, setMeta] = useState<ModelGatewayMetaResponse>();
  const [account, setAccount] = useState<ModelGatewayAccountResponse>();
  const [loading, setLoading] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [error, setError] = useState(false);
  const refresh = useCallback(async () => {
    setLoading(true); setError(false);
    const [nextMeta, nextAccount] = await Promise.allSettled([
      ipcBridge.modelGateway.providerMeta.invoke({ provider_id: provider.id }),
      ipcBridge.modelGateway.account.invoke({ provider_id: provider.id }),
    ]);
    if (nextMeta.status === 'fulfilled') setMeta(nextMeta.value); else setMeta(undefined);
    if (nextAccount.status === 'fulfilled') setAccount(nextAccount.value); else setAccount(undefined);
    setError(nextMeta.status === 'rejected' || nextAccount.status === 'rejected');
    setLoading(false);
  }, [provider.id, provider.base_url, provider.has_credentials]);
  useEffect(() => { void refresh(); }, [refresh]);
  const sync = async () => {
    setSyncing(true); setSyncNotice(undefined);
    try {
      const result = await ipcBridge.modelGateway.sync.invoke({ provider_id: provider.id });
      await onChanged();
      setSyncNotice({ type: 'success', text: t('settings.modelGateway.syncComplete', { added: result.added, updated: result.updated }) });
      await refresh();
    } catch { setSyncNotice({ type: 'error', text: t('settings.modelGateway.syncFailed') }); }
    finally { setSyncing(false); }
  };
  const unknown = t('settings.modelGateway.unknown');
  const purchaseUrl = gatewayExternalUrl(meta?.operator.purchase_url);
  return (
    <div className='my-8px flex flex-col gap-12px' data-model-gateway-details>
      {meta && <GatewayOperator meta={meta} />}
      <div className='flex flex-wrap gap-8px'>
        <Button size='small' onClick={() => void refresh()} loading={loading}>{t('settings.modelGateway.refreshAccount')}</Button>
        <Button size='small' onClick={() => void sync()} loading={syncing} disabled={loading}>{t('settings.modelGateway.syncCatalog')}</Button>
        {purchaseUrl && <Button size='small' type='primary' onClick={() => void ipcBridge.shell.openExternal.invoke(purchaseUrl)}>{t('settings.modelGateway.recharge')}</Button>}
      </div>
      {error && <Alert type='warning' content={t('settings.modelGateway.accountFailed')} />}
      {syncNotice && <Alert type={syncNotice.type} content={syncNotice.text} />}
      {account && <dl className='grid grid-cols-[minmax(100px,auto)_minmax(0,1fr)] gap-x-16px gap-y-8px m-0 p-12px rd-8px bg-[var(--color-bg-1)] text-13px'>
        <dt className='text-t-secondary'>{t('settings.modelGateway.plan')}</dt><dd className='m-0 break-words'>{account.plan?.name ?? t('settings.modelGateway.noPlan')}</dd>
        <dt className='text-t-secondary'>{t('settings.modelGateway.balance')}</dt><dd className='m-0'>{gatewayMoney(account.balance.amount, account.balance.currency, i18n.language)}</dd>
        <dt className='text-t-secondary'>{t('settings.modelGateway.planEnd')}</dt><dd className='m-0'>{gatewayDate(account.plan?.period_end, i18n.language, unknown)}</dd>
        <dt className='text-t-secondary'>{t('settings.modelGateway.usage')}</dt><dd className='m-0'>{account.plan ? `${gatewayInteger(account.plan.quota.used, i18n.language, unknown)} / ${gatewayInteger(account.plan.quota.total, i18n.language, unknown)} ${account.plan.quota.unit}` : unknown}</dd>
        <dt className='text-t-secondary'>{t('settings.modelGateway.keyName')}</dt><dd className='m-0 break-words'>{account.key.name}</dd>
        <dt className='text-t-secondary'>{t('settings.modelGateway.keyEnd')}</dt><dd className='m-0'>{gatewayDate(account.key.expires_at, i18n.language, t('settings.modelGateway.noExpiry'))}</dd>
        <dt className='text-t-secondary'>{t('settings.modelGateway.keyRemaining')}</dt><dd className='m-0'>{gatewayInteger(account.key.remaining_quota, i18n.language, unknown)} {account.key.quota_unit}</dd>
      </dl>}
      <div className='text-12px text-t-secondary'>{t('settings.modelGateway.syncHint')}</div>
    </div>
  );
};
export default ModelGatewayDetails;
