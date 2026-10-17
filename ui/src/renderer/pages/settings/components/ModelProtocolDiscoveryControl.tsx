import { ipcBridge } from '@/common';
import type { DiscoverModelProtocolsResponse } from '@/common/protocolBindings/DiscoverModelProtocolsResponse';
import type { ModelProtocolCandidate } from '@/common/protocolBindings/ModelProtocolCandidate';
import type { ProviderModelCapabilityInput } from '@/common/types/provider/providerModel';
import { Button, Spin, Tag } from '@arco-design/web-react';
import { IconLoading } from '@arco-design/web-react/icon';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { healthFailureHeadline } from '@/renderer/components/settings/SettingsModal/contents/healthFailureHeadline';
import { modelProtocolDiscoveryContextSnapshot, modelProtocolDiscoverySnapshot, type ModelProtocolDiscoveryContext } from './modelProtocolDiscovery';
import { capabilityDraftFromResponse } from './providerModelAdvanced';
import { isOpenAIChatProtocol } from './chatProtocolTransition';

interface Props {
  model: string;
  committed: boolean;
  attemptState?: { started: boolean };
  applyRecommendedDefaults: boolean;
  capability?: ProviderModelCapabilityInput;
  snapshot: string;
  context: ModelProtocolDiscoveryContext;
  preferredProtocol?: string;
  onSelect: (snapshot: string, selected: ProviderModelCapabilityInput, source: 'automatic' | 'manual') => void;
  onPendingChange?: (pending: boolean) => void;
  onConfigureProtocol?: (protocol: string, suspendNativeParams: boolean) => void;
}
interface Completed { key: string; result?: DiscoverModelProtocolsResponse; error?: string }
const protocolLabel = (protocol: string): string => protocol === 'openai.responses'
  ? 'Responses' : protocol === 'openai.chat_text' ? 'Chat Completions' : protocol;
const keyFor = (snapshot: string, preferred: string | undefined, revision: number): string => JSON.stringify([snapshot, preferred ?? '', revision]);

/** Verify committed model IDs. Results and deduplication stay in this editor. */
export default function ModelProtocolDiscoveryControl({ model, committed, attemptState: sharedAttemptState, applyRecommendedDefaults, capability, snapshot, context, preferredProtocol, onSelect, onPendingChange, onConfigureProtocol }: Props) {
  const { t } = useTranslation();
  const [checkingKey, setCheckingKey] = useState<string>();
  const [completed, setCompleted] = useState<Completed>();
  const [retry, setRetry] = useState(0);
  const localAttemptState = useRef({ started: false });
  const attemptState = sharedAttemptState ?? localAttemptState.current;
  const requestedKey = useRef<string | undefined>(undefined);
  const cache = useRef(new Map<string, Completed>());
  const contextSnapshot = modelProtocolDiscoveryContextSnapshot(context);
  const contextVersion = useRef({ snapshot: contextSnapshot, revision: 0 });
  if (contextVersion.current.snapshot !== contextSnapshot) {
    contextVersion.current = { snapshot: contextSnapshot, revision: contextVersion.current.revision + 1 };
    cache.current.clear(); // Never retain old credential snapshots in deduplication keys.
  }
  const revision = contextVersion.current.revision;
  const disclosureIdentity = JSON.stringify([model.trim(), capability?.task, revision]);
  const [disclosure, setDisclosure] = useState({ identity: disclosureIdentity, settings: false, diagnostics: false });
  const settingsOpen = disclosure.identity === disclosureIdentity && disclosure.settings;
  const diagnosticsOpen = settingsOpen && disclosure.diagnostics;
  const key = keyFor(snapshot, preferredProtocol, revision);
  // A click authorizes this uninterrupted draft only, including during debounce.
  if (requestedKey.current !== key) requestedKey.current = undefined;
  const current = useRef({ key, onSelect, onPendingChange });
  current.current = { key, onSelect, onPendingChange };
  const ready = Boolean(committed && model.trim() && capability &&
    (context.providerId || context.credentials !== undefined) && (context.providerId || context.baseUrl.trim() || context.bedrockConfig));
  const checking = ready && checkingKey === key;

  useEffect(() => {
    setDisclosure({ identity: disclosureIdentity, settings: false, diagnostics: false });
  }, [disclosureIdentity]);

  useEffect(() => {
    if (!ready || !capability) { setCheckingKey(undefined); return; }
    const cached = cache.current.get(key);
    if (cached) { setCompleted(cached); setCheckingKey(undefined); return; }
    // Each scene gets one automatic attempt. Later draft edits only invalidate
    // its result; another network request requires an explicit button click.
    if (requestedKey.current !== key && attemptState.started) { setCheckingKey(undefined); return; }
    let active = true;
    const controller = new AbortController();
    setCheckingKey(key);
    current.current.onPendingChange?.(true);
    const timer = setTimeout(() => {
      requestedKey.current = undefined;
      attemptState.started = true;
      void (async () => {
        try {
          const body = { model: model.trim(), capability, apply_recommended_defaults: applyRecommendedDefaults,
            ...(preferredProtocol ? { preferred_protocol: preferredProtocol } : {}) };
          const result = context.providerId
            ? await ipcBridge.mode.discoverProviderModelProtocols.invoke({ ...body, provider_id: context.providerId }, controller.signal)
            : await ipcBridge.mode.discoverModelProtocols.invoke({ ...body,
                platform: context.platform, base_url: context.baseUrl.trim(), auth_scheme: context.authScheme,
                credentials: context.credentials ?? {}, connections: context.connections,
                ...(context.bedrockConfig ? { bedrock_config: context.bedrockConfig } : {}),
              }, controller.signal);
          if (!active || current.current.key !== key) return;
          const selection = result.candidates.find(candidate => candidate.protocol === result.selected_protocol &&
            candidate.status === 'available' && candidate.capability);
          const selected = selection && result.selected_capability?.protocol === selection.protocol
            ? result.selected_capability : undefined;
          const entry: Completed = { key, result };
          cache.current.set(key, entry);
          // Candidate configurations are exact backend results. Remember the
          // resulting snapshots so applying one never starts another call.
          // Applied parameters become user-owned, so these snapshots disable
          // further default injection even when the starting draft allowed it.
          for (const candidate of result.candidates) if (candidate.status === 'available' && candidate.capability) {
            const candidateSnapshot = modelProtocolDiscoverySnapshot(model, capabilityDraftFromResponse(candidate.capability));
            for (const preferred of [undefined, candidate.protocol]) {
              const candidateKey = keyFor(candidateSnapshot, preferred, revision);
              cache.current.set(candidateKey, { key: candidateKey, result });
            }
          }
          if (selected) {
            const selectedSnapshot = modelProtocolDiscoverySnapshot(model, capabilityDraftFromResponse(selected));
            const selectedKey = keyFor(selectedSnapshot, preferredProtocol === selected.protocol ? preferredProtocol : undefined, revision);
            const applied = { key: selectedKey, result };
            cache.current.set(selectedKey, applied); setCompleted(applied);
            current.current.onSelect(snapshot, selected, 'automatic');
          } else setCompleted(entry);
        } catch (reason) {
          if (!active || controller.signal.aborted || current.current.key !== key) return;
          const entry = { key, error: reason instanceof Error ? reason.message : String(reason) };
          cache.current.set(key, entry); setCompleted(entry);
        } finally {
          if (active) { setCheckingKey(undefined); current.current.onPendingChange?.(false); }
        }
      })();
    }, 300);
    return () => {
      active = false; clearTimeout(timer); controller.abort(); current.current.onPendingChange?.(false);
    };
  }, [key, ready, retry, attemptState]);

  const displayed = completed?.key === key ? completed : undefined;
  const result = displayed?.result;
  const available = result?.candidates.filter(candidate => candidate.status === 'available') ?? [];
  const diagnostics = result?.candidates.filter(candidate => candidate.status !== 'available') ?? [];
  const selected = available.find(candidate => candidate.protocol === capability?.protocol);
  const needsReview = available.length === 0 && diagnostics.some(candidate => candidate.status === 'blocked' || candidate.status === 'incompatible');
  const isRequestFailure = (candidate: ModelProtocolCandidate) => candidate.status === 'inconclusive' && Boolean(candidate.error_kind);
  const failure = available.length > 0 ? undefined
    : diagnostics.find(candidate => candidate.protocol === capability?.protocol && isRequestFailure(candidate)) ?? diagnostics.find(isRequestFailure);
  const status = !committed ? t('settings.modelProtocolDiscovery.awaitCommit')
    : !ready ? t('settings.modelProtocolDiscovery.needsConfiguration')
    : checking ? t('settings.modelProtocolDiscovery.checking')
    : !displayed ? t(attemptState.started ? 'settings.modelProtocolDiscovery.changed' : 'settings.modelProtocolDiscovery.waiting')
    : available.length > 0 ? t(selected?.evidence === 'verified_stream' ? 'settings.modelProtocolDiscovery.connected' : 'settings.modelProtocolDiscovery.configured')
    : failure ? healthFailureHeadline(t, failure)
    : needsReview ? t('settings.modelProtocolDiscovery.reviewConfiguration')
    : t('settings.modelProtocolDiscovery.unavailable');
  const evidenceLabel = (candidate: ModelProtocolCandidate): string => {
    if (candidate.status === 'incompatible') return t('settings.modelProtocolDiscovery.incompatible');
    if (candidate.status === 'blocked') return t('settings.modelProtocolDiscovery.blocked');
    if (candidate.status === 'inconclusive') return t('settings.modelProtocolDiscovery.inconclusive');
    switch (candidate.evidence) {
      case 'verified_stream': return t('settings.modelProtocolDiscovery.verified');
      case 'official_model': return t('settings.modelProtocolDiscovery.documented');
      case 'catalog_task': return t('settings.modelProtocolDiscovery.catalog');
      default: return t('settings.modelProtocolDiscovery.registered');
    }
  };
  const choose = (candidate: ModelProtocolCandidate) => {
    if (candidate.status !== 'available' || !candidate.capability || checking) return;
    const nextSnapshot = modelProtocolDiscoverySnapshot(model, capabilityDraftFromResponse(candidate.capability));
    const nextKey = keyFor(nextSnapshot, candidate.protocol, revision);
    const entry = { key: nextKey, result };
    cache.current.set(nextKey, entry); setCompleted(entry);
    current.current.onSelect(snapshot, candidate.capability, 'manual');
  };
  const check = () => {
    cache.current.delete(key);
    requestedKey.current = key;
    setRetry(value => value + 1);
  };
  return <div className='space-y-6px border-0 border-t border-solid border-[var(--color-border-2)] px-14px py-8px' data-model-protocol-discovery={capability?.task} aria-busy={checking}>
    <div className='flex items-center justify-between gap-8px text-11px leading-4 text-t-secondary'>
      <span className='inline-flex min-w-0 items-center gap-6px' role='status' aria-live='polite' aria-atomic='true'>
        {checking && <Spin size={14} aria-hidden='true' element={<IconLoading className='motion-reduce:!animate-none' style={{ fontSize: 14 }} />} />}
        <span>{status}</span>
      </span>
      <div className='flex shrink-0 items-center gap-4px'>
        <Button size='mini' type='text' disabled={!ready || checking} onClick={check} data-model-protocol-recheck>
          {t('settings.modelProtocolDiscovery.recheck')}
        </Button>
        <Button size='mini' type='text' aria-expanded={settingsOpen} onClick={() => {
          setDisclosure({ identity: disclosureIdentity, settings: !settingsOpen, diagnostics: false });
        }}>{t('settings.modelProtocolDiscovery.connectionSettings')}</Button>
      </div>
    </div>
    {result && <div className='space-y-6px pb-2px text-11px leading-4 text-t-secondary' data-model-protocol-results>
      <span>{t('settings.modelProtocolDiscovery.title')}</span>
      {result.candidates.map(candidate => <div key={candidate.protocol} className='flex flex-wrap items-center gap-6px break-words' data-model-protocol-candidate={candidate.protocol}>
        <Button size='mini' disabled={checking || candidate.status !== 'available' || !candidate.capability}
          aria-pressed={capability?.protocol === candidate.protocol}
          type={capability?.protocol === candidate.protocol ? 'primary' : 'secondary'} onClick={() => choose(candidate)}>
          {protocolLabel(candidate.protocol)}
        </Button>
        <span>{evidenceLabel(candidate)}</span>
        {candidate.recommended && <Tag size='small'>{t('settings.modelProtocolDiscovery.recommended')}</Tag>}
      </div>)}
    </div>}
    {settingsOpen && <div className='space-y-6px pb-2px text-11px leading-4 text-t-secondary' data-model-protocol-settings>
      <div className='flex items-center justify-between gap-8px'>
        <span>{t('settings.modelProtocolDiscovery.title')}</span>
      </div>
      {(displayed?.error || diagnostics.length > 0) && <div className='text-t-secondary'>
        <Button size='mini' type='text' aria-expanded={diagnosticsOpen} onClick={() => {
          setDisclosure({ identity: disclosureIdentity, settings: true, diagnostics: !diagnosticsOpen });
        }}>{t('settings.modelProtocolDiscovery.diagnostics')}</Button>
        {diagnosticsOpen && <div className='max-h-240px overflow-auto' data-model-protocol-diagnostics>
          {displayed?.error && <pre className='m-0 whitespace-pre-wrap break-words pt-4px font-inherit'>{displayed.error}</pre>}
          {diagnostics.map(candidate => <div key={candidate.protocol} className='break-words pt-4px'>
            <span>{protocolLabel(candidate.protocol)}</span>
            {candidate.message || candidate.error_kind ? <pre className='m-0 whitespace-pre-wrap break-words font-inherit'>{candidate.message || candidate.error_kind}</pre> : null}
            {candidate.status === 'blocked' && candidate.protocol !== capability?.protocol && onConfigureProtocol && (() => {
              const nativeSwitch = !(isOpenAIChatProtocol(capability?.protocol ?? '') && isOpenAIChatProtocol(candidate.protocol));
              return <div className='space-y-4px pt-4px' data-configure-protocol={candidate.protocol}>
                {nativeSwitch && <div>{t('settings.modelProtocolDiscovery.nativeSwitchImpact')}</div>}
                <Button size='mini' disabled={checking} onClick={() => onConfigureProtocol(candidate.protocol, nativeSwitch)}>
                  {t(nativeSwitch ? 'settings.modelProtocolDiscovery.configureNative' : 'settings.modelProtocolDiscovery.configure', { protocol: protocolLabel(candidate.protocol) })}
                </Button>
              </div>;
            })()}
          </div>)}
        </div>}
      </div>}
    </div>}
  </div>;
}
