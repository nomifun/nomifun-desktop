/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { ConversationContextValue } from '@/renderer/hooks/context/ConversationContext';
import type { ModelFailureReason } from '@/common/chat/providerDiagnostic';
import { gatewayExternalUrl } from '@/renderer/pages/settings/components/modelGatewayForm';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

export const gatewayActionError = (code?: string): boolean => [
  'USER_LLM_PROVIDER_BILLING_REQUIRED', 'USER_LLM_PROVIDER_AUTH_FAILED', 'USER_LLM_PROVIDER_RATE_LIMITED',
].includes(code ?? '');

/** Account actions are resolved from the selected provider's trusted metadata, never from transcript prose. */
const GatewayBillingAction: React.FC<{ code?: string; reason?: ModelFailureReason; providerId?: string; model?: ConversationContextValue['currentModel'] }> = ({ code, reason, providerId, model }) => {
  const { t } = useTranslation();
  const [link, setLink] = useState<{ identity: string; url: string }>();
  const purchase = code === 'USER_LLM_PROVIDER_BILLING_REQUIRED'
    && (reason === 'insufficient_balance' || reason === 'subscription_expired');
  const identity = `${model?.id ?? ''}:${providerId ?? ''}:${code ?? ''}:${reason ?? ''}`;
  const url = link?.identity === identity ? link.url : undefined;
  const gateway = model?.platform === 'nomifun-model-gateway' && gatewayActionError(code);
  const exactProvider = gateway && providerId === model?.id;
  useEffect(() => {
    setLink(undefined);
    if (!exactProvider || !model) return;
    let active = true;
    void ipcBridge.modelGateway.providerMeta.invoke({ provider_id: model.id }).then((meta) => {
      if (!active) return;
      const url = gatewayExternalUrl(purchase ? meta.operator.purchase_url : meta.operator.console_url);
      if (url) setLink({ identity, url });
    }).catch(() => { /* Model settings remains available when the gateway is unreachable. */ });
    return () => { active = false; };
  }, [code, exactProvider, model?.id, identity, purchase]);
  if (!gatewayActionError(code) || (model && !gateway)) return null;
  return <div className='flex flex-wrap items-center gap-8px mt-8px' data-gateway-billing-action>
    {url && <button type='button' className='message-error-note__retry' onClick={() => void ipcBridge.shell.openExternal.invoke(url)}>{t(purchase
      ? reason === 'subscription_expired' ? 'conversation.agentError.renewSubscription' : 'settings.modelGateway.recharge'
      : 'settings.modelGateway.links.console')}</button>}
    <a href='#/settings/model' className='message-error-note__retry'>{t('settings.modelGateway.openSettings')}</a>
  </div>;
};
export default GatewayBillingAction;
