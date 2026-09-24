import type {
  IProvider,
  ModelTechnicalCapability,
  ModelTrait,
  TProviderWithModel,
} from '@/common/config/storage';
import { compositeKey } from '@/common/utils/compositeKey';
import { modelDisplayLabel } from '@/common/utils/modelPresentation';
import { capabilityOf } from '@/common/utils/providerModels';
import { exactChatHealthDotColor } from './chatModelHealth';
import type { SessionReasoningEffort } from '@/common/types/reasoningEffort';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { Button, Dropdown, Menu } from '@arco-design/web-react';
import { Brain, Check, Down, Plus } from '@icon-park/react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

type ChatModelGroup = { provider: IProvider; models: string[] };

export const filterCompatibleChatModelGroups = (
  groups: readonly ChatModelGroup[],
  requiredTraits: readonly ModelTrait[] = [],
  requiredTechnicalCapabilities: readonly ModelTechnicalCapability[] = []
): ChatModelGroup[] =>
  groups
    .map(group => ({
      ...group,
      models: group.models.filter(name => {
        const capability = capabilityOf(group.provider, name, 'chat');
        const traits = capability?.traits ?? [];
        const unsupported = capability?.health?.unsupported_technical_capabilities ?? [];
        return requiredTraits.every(trait => traits.includes(trait))
          && requiredTechnicalCapabilities.every(technical => !unsupported.includes(technical));
      }),
    }))
    .filter(group => group.models.length > 0);

/** Rendering is shared; the caller owns eligibility and persistence. */
export default function ChatModelSelector({ providers, currentModel, getAvailableModels, onSelectModel,
  disabled = false, compact = false, className = '', readOnlyLabel, testId = 'chat-model-selector',
  requiredTraits = [], requiredTechnicalCapabilities = [], popupVisible, onPopupVisibleChange,
  reasoningEffort, reasoningEffortSupported = false, reasoningEffortDisabled = false,
  onReasoningEffortChange,
}: {
  providers: IProvider[];
  currentModel?: TProviderWithModel;
  getAvailableModels(provider: IProvider): string[];
  onSelectModel(provider: IProvider, model: string): Promise<void>;
  disabled?: boolean;
  compact?: boolean;
  className?: string;
  readOnlyLabel?: string;
  testId?: string;
  requiredTraits?: readonly ModelTrait[];
  requiredTechnicalCapabilities?: readonly ModelTechnicalCapability[];
  popupVisible?: boolean;
  onPopupVisibleChange?: (visible: boolean) => void;
  reasoningEffort?: SessionReasoningEffort;
  reasoningEffortSupported?: boolean;
  reasoningEffortDisabled?: boolean;
  onReasoningEffortChange?: (value: SessionReasoningEffort | undefined) => Promise<void> | void;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [reasoningPopupVisible, setReasoningPopupVisible] = useState(false);
  const providerLabel = useModelSelectorProviderLabel();
  const { data: configuredProviders } = useProvidersQuery();
  const provider = configuredProviders?.find(item => item.id === currentModel?.id) ?? providers.find(item => item.id === currentModel?.id);
  const model = provider?.models.find(item => item.model === currentModel?.use_model);
  const label = readOnlyLabel ?? (currentModel?.use_model
    ? modelDisplayLabel(currentModel.use_model, model?.display_name)
    : t('conversation.welcome.selectModel'));
  const showReasoning = Boolean(onReasoningEffortChange)
    && (reasoningEffortSupported || reasoningEffort !== undefined);
  const reasoningLabel = t(`conversation.reasoningEffort.${reasoningEffort ?? 'auto'}`);
  const reasoningAriaLabel = `${t('conversation.reasoningEffort.label')}: ${reasoningLabel}`;
  const groups = providers.filter(item => item.enabled !== false)
    .map(item => ({ provider: item, models: getAvailableModels(item) })).filter(item => item.models.length);
  const modelTrigger = <Button data-testid={testId} data-readonly={disabled ? 'true' : undefined}
    className={`sendbox-model-btn header-model-btn nomi-sendbox-model-btn min-w-0 ${showReasoning ? 'sendbox-model-segment' : ''} ${compact ? '!max-w-[120px]' : showReasoning ? '!max-w-[232px]' : '!max-w-[280px]'} ${className}`}
    shape='round' size='small' aria-label={label} style={disabled ? { cursor: 'default' } : undefined}>
    <span className='flex items-center gap-6px min-w-0'>
      <Brain theme='outline' size={14} fill='currentColor' className='shrink-0' />
      <span className='sendbox-responsive-label block truncate min-w-0'>{label}</span>
      {!disabled && <Down size={12} fill='currentColor' className='sendbox-responsive-chevron shrink-0' />}
    </span>
  </Button>;
  const compatibleGroups = filterCompatibleChatModelGroups(
    groups,
    requiredTraits,
    requiredTechnicalCapabilities
  );
  const modelMenu = <Menu selectedKeys={currentModel
    ? [compositeKey(currentModel.id, currentModel.use_model)]
    : []}>
    {compatibleGroups.length === 0 && <Menu.Item key='no-models' disabled>{t(
      requiredTraits.length > 0 || requiredTechnicalCapabilities.length > 0
        ? 'guid.agentEntries.modelCompatibility.noCompatibleOption'
        : 'settings.noAvailableModels'
    )}</Menu.Item>}
    {compatibleGroups.map(group => <Menu.ItemGroup key={group.provider.id} title={providerLabel(group.provider)}>
      {group.models.map(name => {
        const dot = exactChatHealthDotColor(configuredProviders ?? providers, group.provider.id, name);
        const displayName = group.provider.models.find(item => item.model === name)?.display_name;
        const selected = currentModel?.id === group.provider.id && currentModel.use_model === name;
        return <Menu.Item key={compositeKey(group.provider.id, name)} data-testid={`nomi-model-option-${name}`}
          onClick={() => {
            onPopupVisibleChange?.(false);
            void onSelectModel(group.provider, name).catch(error => console.error('Failed to select chat model:', error));
          }}>
          <div className='flex items-center justify-between gap-16px w-full min-w-[164px]'>
            <span className='flex items-center gap-8px min-w-0'>
              {dot && <span className={`w-6px h-6px rounded-full shrink-0 ${dot}`} />}
              <span className='truncate'>{modelDisplayLabel(name, displayName)}</span>
            </span>
            {selected && <Check size={13} fill='currentColor' className='shrink-0 text-primary-6' />}
          </div>
        </Menu.Item>;
      })}
    </Menu.ItemGroup>)}
    <Menu.Item key='add-model' onClick={() => navigate('/models?section=models')}><Plus size={12} />{t('settings.addModel')}</Menu.Item>
  </Menu>;
  const modelControl = disabled ? modelTrigger : <Dropdown
    trigger='click'
    getPopupContainer={() => document.body}
    popupVisible={popupVisible}
    onVisibleChange={(visible) => {
      if (visible) setReasoningPopupVisible(false);
      onPopupVisibleChange?.(visible);
    }}
    droplist={modelMenu}
  >{modelTrigger}</Dropdown>;

  const reasoningTrigger = showReasoning ? <Button
    data-testid={`${testId}-reasoning-trigger`}
    data-readonly={disabled || reasoningEffortDisabled ? 'true' : undefined}
    className='sendbox-model-btn header-model-btn nomi-sendbox-model-btn sendbox-reasoning-segment shrink-0 !px-8px'
    shape='round'
    size='small'
    aria-label={reasoningAriaLabel}
    style={disabled || reasoningEffortDisabled ? { cursor: 'default' } : undefined}
  >
    <span className='flex items-center gap-4px'>
      <span className='text-12px' data-testid={`${testId}-reasoning-value`}>
        {reasoningLabel}
      </span>
      {!disabled && !reasoningEffortDisabled && <Down size={12} fill='currentColor' />}
    </span>
  </Button> : null;
  const reasoningMenu = showReasoning ? <Menu
    selectedKeys={[`reasoning:${reasoningEffort ?? 'auto'}`]}
  >
    <Menu.ItemGroup key='reasoning-effort' title={t('conversation.reasoningEffort.label')}>
      {([undefined, 'low', 'medium', 'high'] as const).map(effort => {
        const selected = effort === reasoningEffort;
        const fixedEffortUnavailable = effort !== undefined && !reasoningEffortSupported;
        return <Menu.Item
          key={`reasoning:${effort ?? 'auto'}`}
          data-testid={`${testId}-reasoning-${effort ?? 'auto'}`}
          disabled={reasoningEffortDisabled || fixedEffortUnavailable}
          onClick={() => {
            setReasoningPopupVisible(false);
            if (selected) return;
            void Promise.resolve(onReasoningEffortChange?.(effort)).catch(error =>
              console.error('Failed to update session reasoning effort:', error)
            );
          }}
        >
          <div className='flex items-center justify-between gap-16px min-w-[164px]'>
            <span>{t(`conversation.reasoningEffort.${effort ?? 'auto'}`)}</span>
            {selected && <Check size={13} fill='currentColor' className='text-primary-6' />}
          </div>
        </Menu.Item>;
      })}
    </Menu.ItemGroup>
  </Menu> : null;
  const reasoningControl = reasoningTrigger && reasoningMenu
    ? disabled || reasoningEffortDisabled
      ? reasoningTrigger
      : <Dropdown
          trigger='click'
          getPopupContainer={() => document.body}
          popupVisible={reasoningPopupVisible}
          onVisibleChange={(visible) => {
            setReasoningPopupVisible(visible);
            if (visible) onPopupVisibleChange?.(false);
          }}
          droplist={reasoningMenu}
        >{reasoningTrigger}</Dropdown>
    : null;

  return <span
    className={`inline-flex items-center min-w-0 ${showReasoning ? 'sendbox-model-reasoning-group' : ''}`}
    data-testid={`${testId}-controls`}
  >
    {modelControl}
    {reasoningControl}
  </span>;
}
