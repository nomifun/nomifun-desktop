import type { IProvider, TProviderWithModel } from '@/common/config/storage';
import { compositeKey } from '@/common/utils/compositeKey';
import { modelDisplayLabel } from '@/common/utils/modelPresentation';
import { exactChatHealthDotColor } from './chatModelHealth';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { Button, Dropdown, Menu } from '@arco-design/web-react';
import { Brain, Down, Plus } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

/** Rendering is shared; the caller owns eligibility and persistence. */
export default function ChatModelSelector({ providers, currentModel, getAvailableModels, onSelectModel,
  disabled = false, compact = false, className = '', readOnlyLabel, testId = 'chat-model-selector',
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
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const providerLabel = useModelSelectorProviderLabel();
  const { data: configuredProviders } = useProvidersQuery();
  const provider = configuredProviders?.find(item => item.id === currentModel?.id) ?? providers.find(item => item.id === currentModel?.id);
  const model = provider?.models.find(item => item.model === currentModel?.use_model);
  const label = readOnlyLabel ?? (currentModel?.use_model
    ? modelDisplayLabel(currentModel.use_model, model?.display_name)
    : t('conversation.welcome.selectModel'));
  const groups = providers.filter(item => item.enabled !== false)
    .map(item => ({ provider: item, models: getAvailableModels(item) })).filter(item => item.models.length);
  const trigger = <Button data-testid={testId} data-readonly={disabled ? 'true' : undefined}
    className={`sendbox-model-btn header-model-btn nomi-sendbox-model-btn min-w-0 ${compact ? '!max-w-[120px]' : '!max-w-[280px]'} ${className}`}
    shape='round' size='small' aria-label={label} style={disabled ? { cursor: 'default' } : undefined}>
    <span className='flex items-center gap-6px min-w-0'>
      <Brain theme='outline' size={14} fill='currentColor' className='shrink-0' />
      <span className='sendbox-responsive-label block truncate min-w-0'>{label}</span>
      {!disabled && <Down size={12} fill='currentColor' className='sendbox-responsive-chevron shrink-0' />}
    </span>
  </Button>;
  if (disabled) return trigger;
  return <Dropdown trigger='click' droplist={<Menu selectedKeys={currentModel ? [compositeKey(currentModel.id, currentModel.use_model)] : []}>
    {groups.length === 0 && <Menu.Item key='no-models' disabled>{t('settings.noAvailableModels')}</Menu.Item>}
    {groups.map(group => <Menu.ItemGroup key={group.provider.id} title={providerLabel(group.provider)}>
      {group.models.map(name => {
        const dot = exactChatHealthDotColor(configuredProviders ?? providers, group.provider.id, name);
        const displayName = group.provider.models.find(item => item.model === name)?.display_name;
        return <Menu.Item key={compositeKey(group.provider.id, name)} data-testid={`nomi-model-option-${name}`}
          onClick={() => { void onSelectModel(group.provider, name).catch(error => console.error('Failed to select chat model:', error)); }}>
          <div className='flex items-center gap-8px w-full'>
            {dot && <span className={`w-6px h-6px rounded-full shrink-0 ${dot}`} />}
            <span>{modelDisplayLabel(name, displayName)}</span>
          </div>
        </Menu.Item>;
      })}
    </Menu.ItemGroup>)}
    <Menu.Item key='add-model' onClick={() => navigate('/models?section=models')}><Plus size={12} />{t('settings.addModel')}</Menu.Item>
  </Menu>}>{trigger}</Dropdown>;
}
