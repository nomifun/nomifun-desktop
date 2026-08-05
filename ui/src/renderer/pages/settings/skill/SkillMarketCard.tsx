import type { PresetTag } from '@/common/types/agent/presetTypes';
import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import { getAvatarColorClass, normalizeTestId } from './skillPresentation';
import { marketSourceLabel, translateMarketDescription } from './skillMarket';
import { Button, Tag } from '@arco-design/web-react';
import { Plus } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';

type SkillMarketCardProps = {
  item: ISkillMarketItem;
  tagByKey: Map<string, PresetTag>;
  localeKey: string;
  onAdd: (item: ISkillMarketItem) => void;
};

const MAX_VISIBLE_TAGS = 4;

const resolveTagLabel = (tag: PresetTag, localeKey: string): string => tag.label_i18n?.[localeKey] || tag.label;

const MarketSourceBadge: React.FC<{ source: ISkillMarketItem['source'] }> = ({ source }) => {
  const label = marketSourceLabel(source);
  const className = {
    clawhub: '!bg-primary-1 !text-primary-6',
    loophub: '!bg-[rgba(var(--warning-6),0.12)] !text-warning-7',
    skillhub: '!bg-[rgba(var(--success-6),0.1)] !text-success-6',
    skillhub_mcp: '!bg-[rgba(var(--arcoblue-6),0.1)] !text-[rgba(var(--arcoblue-6),1)]',
    mcpworld: '!bg-[rgba(var(--purple-6),0.1)] !text-[rgba(var(--purple-6),1)]',
    clawhub_plugins: '!bg-[rgba(var(--orange-6),0.12)] !text-[rgba(var(--orange-7),1)]',
    skillhub_packages: '!bg-[rgba(var(--cyan-6),0.1)] !text-[rgba(var(--cyan-7),1)]',
  }[source];

  return (
    <Tag
      size='small'
      bordered={false}
      className={`!flex-shrink-0 !text-10px !leading-14px !px-6px !py-0 !rounded-6px ${className}`}
    >
      {label}
    </Tag>
  );
};

const SkillMarketCard: React.FC<SkillMarketCardProps> = ({ item, tagByKey, localeKey, onAdd }) => {
  const { t } = useTranslation();
  const testId = normalizeTestId(item.id);
  const requiresApiKey = item.tags?.includes('requires_api_key') ?? false;
  const noApiKey = item.tags?.includes('no_api_key') ?? false;
  const resolvedTags = [...(item.audience_tags ?? []), ...(item.scenario_tags ?? [])]
    .map((key) => tagByKey.get(key))
    .filter((tag): tag is PresetTag => Boolean(tag));
  const rawTags = (item.tags ?? []).filter((tag) => !tagByKey.has(tag) && tag !== 'requires_api_key' && tag !== 'no_api_key');
  const visibleResolvedTags = resolvedTags.slice(0, MAX_VISIBLE_TAGS);
  const visibleRawTags = resolvedTags.length === 0 ? rawTags.slice(0, MAX_VISIBLE_TAGS) : [];
  const totalTagCount = resolvedTags.length > 0 ? resolvedTags.length : rawTags.length;
  const overflowCount = Math.max(0, totalTagCount - MAX_VISIBLE_TAGS);
  const description = translateMarketDescription(item.description, item, localeKey);

  return (
    <div
      data-testid={`skill-market-card-${testId}`}
      className={[
        'group relative flex flex-col rounded-16px border border-solid p-14px outline-none',
        'transition-all duration-180',
        'border-[var(--color-border-2)] bg-[var(--color-bg-2)] hover:border-[var(--color-primary-light-4)] hover:shadow-[0_4px_16px_rgba(0,0,0,0.06)]',
      ].join(' ')}
    >
      <Button
        size='mini'
        type='primary'
        data-testid={`btn-add-market-skill-${testId}`}
        className='!absolute !right-12px !top-12px !rounded-[100px] !h-26px !px-10px !text-12px'
        icon={<Plus theme='outline' size={12} strokeWidth={3} />}
        onClick={() => onAdd(item)}
      >
        {t('common.add', { defaultValue: 'Add' })}
      </Button>

      <div className='flex items-start gap-10px pr-68px'>
        <div
          className={`flex-shrink-0 w-36px h-36px rounded-10px flex items-center justify-center font-bold text-13px shadow-sm ${getAvatarColorClass(item.name)}`}
          title={`#${item.rank || '-'}`}
        >
          {item.rank ? `#${item.rank}` : item.name.charAt(0).toUpperCase()}
        </div>
        <div className='min-w-0 flex-1 pt-2px'>
          <div className='flex items-center gap-6px min-w-0 flex-wrap'>
            <span
              className='truncate max-w-full text-14px font-medium leading-20px text-[var(--color-text-1)]'
              title={item.name}
            >
              {item.name}
            </span>
            <MarketSourceBadge source={item.source} />
            {requiresApiKey && (
              <Tag size='small' bordered={false} className='!bg-[rgba(var(--warning-6),0.12)] !text-warning-7 !rounded-6px !text-10px'>
                {t('settings.market.requiresApi', { defaultValue: 'Needs API' })}
              </Tag>
            )}
            {!requiresApiKey && noApiKey && (
              <Tag size='small' bordered={false} className='!bg-[rgba(var(--success-6),0.1)] !text-success-6 !rounded-6px !text-10px'>
                {t('settings.market.noApi', { defaultValue: 'No API' })}
              </Tag>
            )}
          </div>
          {item.stats && <div className='mt-2px text-11px text-[var(--color-text-3)] truncate'>{item.stats}</div>}
        </div>
      </div>

      <div
        className='mt-10px text-12px leading-18px text-[var(--color-text-3)] min-h-[36px]'
        title={item.description || undefined}
        style={{
          display: '-webkit-box',
          WebkitLineClamp: 2,
          WebkitBoxOrient: 'vertical',
          overflow: 'hidden',
        }}
      >
        {description || t('settings.skillsMarket.noDescription', { defaultValue: '暂无描述。' })}
      </div>

      {(visibleResolvedTags.length > 0 || visibleRawTags.length > 0) && (
        <div className='mt-12px flex flex-wrap items-center gap-6px'>
          {visibleResolvedTags.map((tag) => (
            <span
              key={tag.key}
              className='inline-flex items-center rounded-[12px] px-8px py-1px text-11px leading-16px bg-[var(--color-fill-2)] text-[var(--color-text-2)] border border-solid border-[var(--color-border-2)]'
            >
              {resolveTagLabel(tag, localeKey)}
            </span>
          ))}
          {visibleRawTags.map((tag) => (
            <span
              key={tag}
              className='inline-flex items-center rounded-[12px] px-8px py-1px text-11px leading-16px bg-[var(--color-fill-2)] text-[var(--color-text-2)] border border-solid border-[var(--color-border-2)]'
            >
              {tag}
            </span>
          ))}
          {overflowCount > 0 && (
            <span className='inline-flex items-center rounded-[12px] px-7px py-1px text-11px leading-16px text-[var(--color-text-3)]'>
              +{overflowCount}
            </span>
          )}
        </div>
      )}

      <div className='mt-auto pt-10px flex min-w-0 items-center justify-between gap-10px'>
        <span className='truncate text-11px text-[var(--color-text-3)] font-mono' title={item.install_command}>
          {item.install_command}
        </span>
      </div>
    </div>
  );
};

export default SkillMarketCard;
