/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { ConversationContextValue } from '@/renderer/hooks/context/ConversationContext';
import { gatewayExternalUrl } from '@/renderer/pages/settings/components/modelGatewayForm';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

export const gatewayActionError = (code?: string): boolean => [
  'USER_LLM_PROVIDER_BILLING_REQUIRED', 'USER_LLM_PROVIDER_AUTH_FAILED', 'USER_LLM_PROVIDER_RATE_LIMITED',
].includes(code ?? '');

/** Account actions are resolved from the selected provider's trusted metadata, never from transcript prose. */
const GatewayBillingAction: React.FC<{ code?: string; model?: ConversationContextValue['currentModel'] }> = ({ code, model }) => {
  const { t } = useTranslation();
  const [link, setLink] = useState<{ identity: string; url: string }>();
  const identity = `${model?.id ?? ''}:${code ?? ''}`;
  const url = link?.identity === identity ? link.url : undefined;
  const gateway = model?.platform === 'nomifun-model-gateway' && gatewayActionError(code);
  useEffect(() => {
    setLink(undefined);
    if (!gateway || !model) return;
    let active = true;
    void ipcBridge.modelGateway.providerMeta.invoke({ provider_id: model.id }).then((meta) => {
      if (!active) return;
      const url = gatewayExternalUrl(code === 'USER_LLM_PROVIDER_BILLING_REQUIRED' ? meta.operator.purchase_url : meta.operator.console_url);
      if (url) setLink({ identity, url });
    }).catch(() => { /* Model settings remains available when the gateway is unreachable. */ });
    return () => { active = false; };
  }, [code, gateway, model?.id, identity]);
  if (!gatewayActionError(code) || (model && !gateway)) return null;
  return <div className='flex flex-wrap items-center gap-8px mt-8px' data-gateway-billing-action>
    {url && <button type='button' className='message-error-note__retry' onClick={() => void ipcBridge.shell.openExternal.invoke(url)}>{t(code === 'USER_LLM_PROVIDER_BILLING_REQUIRED' ? 'settings.modelGateway.recharge' : 'settings.modelGateway.links.console')}</button>}
    <a href='#/settings/model' className='message-error-note__retry'>{t('settings.modelGateway.openSettings')}</a>
  </div>;
};
export default GatewayBillingAction;
