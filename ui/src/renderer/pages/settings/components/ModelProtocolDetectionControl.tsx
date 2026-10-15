import { ipcBridge } from '@/common';
import type { DetectChatProtocolResponse } from '@/common/protocolBindings/DetectChatProtocolResponse';
import type { ProviderModelCapabilityInput } from '@/common/types/provider/providerModel';
import { Button } from '@arco-design/web-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { chatProtocolDetectionContextSnapshot, chatProtocolDetectionSnapshot, isDetectableChatProtocol, type ModelProtocolDetectionContext } from './modelProtocolDetection';
import { capabilityDraftFromResponse } from './providerModelAdvanced';

interface Props {
  model: string;
  capability?: ProviderModelCapabilityInput;
  snapshot: string;
  context: ModelProtocolDetectionContext;
  onSelect: (snapshot: string, selected: ProviderModelCapabilityInput) => void;
  onPendingChange?: (pending: boolean) => void;
}

const protocolLabel = (protocol: string): string => protocol === 'openai.responses'
  ? 'Responses' : protocol === 'openai.chat_text' ? 'Chat Completions' : protocol;

/** A paid model check is issued only in this explicit click handler. */
export default function ModelProtocolDetectionControl({ model, capability, snapshot, context, onSelect, onPendingChange }: Props) {
  const { t } = useTranslation();
  const [checking, setChecking] = useState(false);
  const [completed, setCompleted] = useState<{ result: DetectChatProtocolResponse; snapshot: string; context: string }>();
  const [error, setError] = useState('');
  const [stale, setStale] = useState(false);
  const mounted = useRef(true);
  const inFlight = useRef(false);
  const pendingChange = useRef(onPendingChange);
  pendingChange.current = onPendingChange;
  useEffect(() => { mounted.current = true; return () => {
    mounted.current = false;
    if (inFlight.current) pendingChange.current?.(false);
  }; }, []);
  const contextSnapshot = chatProtocolDetectionContextSnapshot(context);
  const current = useRef({ snapshot, context: contextSnapshot, onSelect });
  current.current = { snapshot, context: contextSnapshot, onSelect };
  const outdated = stale || (completed !== undefined && (completed.snapshot !== snapshot || completed.context !== contextSnapshot));
  const result = outdated ? undefined : completed?.result;
  const ready = Boolean(model.trim() && capability && isDetectableChatProtocol(capability.protocol) && context.baseUrl.trim() &&
    (context.providerId || context.credentials !== undefined));

  const detect = async () => {
    if (!ready || !capability || inFlight.current) return;
    inFlight.current = true;
    pendingChange.current?.(true);
    const started = current.current;
    setChecking(true); setCompleted(undefined); setError(''); setStale(false);
    try {
      const detected = context.providerId
        ? await ipcBridge.mode.detectProviderChatProtocol.invoke({ provider_id: context.providerId, model: model.trim(), capability })
        : await ipcBridge.mode.detectChatProtocol.invoke({
            platform: context.platform, base_url: context.baseUrl.trim(), auth_scheme: context.authScheme,
            credentials: context.credentials ?? {}, connections: context.connections,
            model: model.trim(), capability,
          });
      // Local edits and credential/address changes revoke permission to apply
      // this result. No background retry or automatic paid re-probe occurs.
      if (!mounted.current) return;
      if (current.current.snapshot !== started.snapshot || current.current.context !== started.context) {
        setStale(true); return;
      }
      const selectionVerified = detected.selected_capability && detected.selected_protocol === detected.selected_capability.protocol &&
        detected.attempts.some(attempt => attempt.protocol === detected.selected_protocol && attempt.status === 'verified');
      setCompleted({ result: detected, context: started.context,
        snapshot: selectionVerified && detected.selected_capability
          ? chatProtocolDetectionSnapshot(model, capabilityDraftFromResponse(detected.selected_capability)) : started.snapshot });
      if (selectionVerified && detected.selected_capability) {
        current.current.onSelect(started.snapshot, detected.selected_capability);
      }
    } catch (reason) {
      if (!mounted.current) return;
      if (current.current.snapshot !== started.snapshot || current.current.context !== started.context) setStale(true);
      else setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      inFlight.current = false;
      if (mounted.current) { setChecking(false); pendingChange.current?.(false); }
    }
  };

  return <div className='space-y-6px border-0 border-t border-solid border-[var(--color-border-2)] px-14px py-10px' data-model-protocol-detection>
    <Button size='small' loading={checking} disabled={!ready || checking} onClick={() => void detect()}>
      {t('settings.modelProtocolDetection.action', { defaultValue: '检测并选择可用协议' })}
    </Button>
    <div className='text-11px leading-4 text-t-tertiary'>
      {t('settings.modelProtocolDetection.hint', {
        defaultValue: '使用当前模型和参数发送短请求，最多验证两种协议，可能产生少量费用；仅在点击时检测，结果只写入草稿，需保存后生效。',
      })}
    </div>
    {!ready && <div className='text-11px text-t-secondary'>
      {t('settings.modelProtocolDetection.needsConfiguration', { defaultValue: '填写模型、地址和凭据，并修正参数 JSON 后可检测。' })}
    </div>}
    <div className='space-y-4px text-11px leading-4 text-t-secondary' role='status' aria-live='polite'>
      {outdated && <div>{t('settings.modelProtocolDetection.stale', { defaultValue: '配置已更改，先前检测结果已失效；请按需重新检测。' })}</div>}
      {error && <div className='break-words text-danger-6'>{error}</div>}
      {result && <>
        <div>{result.selected_protocol
          ? t('settings.modelProtocolDetection.selected', { defaultValue: '已验证并选择 {{protocol}}；请保存配置。', protocol: protocolLabel(result.selected_protocol) })
          : t('settings.modelProtocolDetection.unavailable', { defaultValue: '尚未找到可用协议；原配置保持不变，请检查下方结果。' })}</div>
        {result.attempts.map(attempt => <div key={attempt.protocol} className='break-words' data-model-protocol-attempt={attempt.protocol}>
          {protocolLabel(attempt.protocol)} · {attempt.status === 'verified'
            ? t('settings.modelProtocolDetection.verified', { defaultValue: '模型调用通过' })
            : attempt.status === 'incompatible'
              ? t('settings.modelProtocolDetection.incompatible', { defaultValue: '不兼容' })
              : t('settings.modelProtocolDetection.failed', { defaultValue: '检测未完成' })}
          {attempt.message || attempt.error_kind ? `：${attempt.message || attempt.error_kind}` : ''}
        </div>)}
      </>}
    </div>
  </div>;
}
