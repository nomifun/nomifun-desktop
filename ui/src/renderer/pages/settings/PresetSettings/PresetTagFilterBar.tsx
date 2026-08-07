/**
 * PresetTagFilterBar — Compact audience / scenario facet controls.
 *
 * The library variant follows the knowledge-library toolbar: dropdown facets
 * stay on the primary row and active selections are echoed below. The drawer
 * keeps the denser inline-chip treatment used by the editor.
 */
import type { PresetTag, PresetTagDimension } from '@/common/types/agent/presetTypes';
import type { PresetTagId } from '@/common/types/ids';
import type { TagFilterState } from './presetUtils';
import { Dropdown, Menu, Tooltip } from '@arco-design/web-react';
import { Check, CloseSmall, Down, SettingTwo } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import filterBarStyles from './PresetTagFilterBar.module.css';

type PresetTagFilterBarProps = {
  audienceTags: PresetTag[];
  scenarioTags: PresetTag[];
  value: TagFilterState;
  onChange: (next: TagFilterState) => void;
  localeKey: string;
  onManageTags: () => void;
  variant?: 'default' | 'drawer';
  className?: string;
  hideManageTags?: boolean;
  manageTagsInlineIcon?: boolean;
  actions?: React.ReactNode;
};

const resolveTagLabel = (tag: PresetTag, localeKey: string): string => tag.label_i18n?.[localeKey] || tag.label;

const ToolbarSelect: React.FC<{
  label: string;
  value: string;
  menu: React.ReactNode;
}> = ({ label, value, menu }) => (
  <Dropdown trigger='click' position='bl' droplist={menu}>
    <div
      role='button'
      tabIndex={0}
      aria-label={`${label}：${value}`}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          event.currentTarget.click();
        }
      }}
      className={[
        'inline-flex h-34px min-w-148px box-border items-center justify-between gap-8px rounded-9px px-10px',
        'border border-solid border-[var(--color-border-3)] bg-[var(--color-bg-2)]',
        'text-13px text-[var(--color-text-1)] cursor-pointer select-none',
        'hover:border-[var(--color-border-4)] hover:bg-[var(--color-fill-2)]',
        'focus-visible:outline-none focus-visible:border-primary-6 transition-colors',
      ].join(' ')}
    >
      <span className='min-w-0 truncate leading-18px'>
        <span className='text-[var(--color-text-2)]'>{label}：</span>
        <span className='font-medium'>{value}</span>
      </span>
      <Down theme='outline' size={12} className='block flex-none leading-none text-[var(--color-text-3)]' />
    </div>
  </Dropdown>
);

const DropdownMenuSurface: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div
    className='overflow-hidden rounded-10px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] shadow-lg'
    style={{ boxShadow: '0 10px 28px rgba(0, 0, 0, 0.14)' }}
  >
    {children}
  </div>
);

const COMPACT_MENU_CLASS = [
  'text-13px',
  '[&_.arco-menu-inner]:!p-4px',
  '[&_.arco-menu-item]:!mb-1px',
  '[&_.arco-menu-item]:!px-9px',
  '[&_.arco-menu-item]:!leading-30px',
].join(' ');

/** Idle/active pill. Active wraps the primary-light triad. */
const FilterChip: React.FC<{
  label: string;
  active: boolean;
  onClick: () => void;
  testId?: string;
  variant?: 'default' | 'drawer';
}> = ({ label, active, onClick, testId, variant = 'default' }) => (
  <div
    role='button'
    tabIndex={0}
    data-testid={testId}
    aria-pressed={active}
    onClick={onClick}
    onKeyDown={(e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        onClick();
      }
    }}
    className={
      variant === 'drawer'
        ? [
            filterBarStyles.drawerFilterChip,
            active ? filterBarStyles.drawerFilterChipActive : '',
          ].filter(Boolean).join(' ')
        : [
            'inline-flex items-center select-none cursor-pointer rounded-[16px] px-12px py-3px text-13px leading-20px',
            'border border-solid transition-all duration-150 whitespace-nowrap',
            active
              ? 'bg-[#151515] text-white border-white font-medium'
              : 'bg-[var(--color-fill-2)] text-[var(--color-text-2)] border-[var(--color-border-2)] hover:bg-[var(--color-fill-3)] hover:text-[var(--color-text-1)]',
          ].join(' ')
    }
  >
    {label}
  </div>
);

const PresetTagFilterBar: React.FC<PresetTagFilterBarProps> = ({
  audienceTags,
  scenarioTags,
  value,
  onChange,
  localeKey,
  onManageTags,
  variant = 'default',
  className,
  hideManageTags = false,
  manageTagsInlineIcon = false,
  actions,
}) => {
  const { t } = useTranslation();
  const isDrawer = variant === 'drawer';
  const allLabel = t('settings.presetTagAll', { defaultValue: 'All' });
  const manageTagsLabel = t('settings.presetManageTags', { defaultValue: 'Manage Tags' });

  const toggle = (dimension: PresetTagDimension, presetTagId: PresetTagId) => {
    const current = value[dimension];
    const next = current.includes(presetTagId)
      ? current.filter((value) => value !== presetTagId)
      : [...current, presetTagId];
    onChange({ ...value, [dimension]: next });
  };

  const clearDimension = (dimension: PresetTagDimension) => {
    onChange({ ...value, [dimension]: [] });
  };

  const renderDrawerRow = (dimension: PresetTagDimension, rowLabel: string, tags: PresetTag[]) => {
    if (tags.length === 0) return null;
    const selected = value[dimension];

    return (
      <div className={isDrawer ? filterBarStyles.drawerFilterRow : 'flex items-start gap-12px'}>
        {/* Left dimension label with accent rail */}
        <div className={isDrawer ? filterBarStyles.drawerFilterLabel : 'flex items-center gap-7px flex-shrink-0 h-26px mt-1px'}>
          <span
            className={isDrawer ? filterBarStyles.drawerFilterRail : 'inline-block w-3px h-12px rounded-[2px] bg-[var(--color-primary-light-3)]'}
            aria-hidden='true'
          />
          <span className={isDrawer ? '' : 'text-12px font-medium text-[var(--color-text-3)] whitespace-nowrap'}>{rowLabel}</span>
        </div>
        <div className={isDrawer ? filterBarStyles.drawerFilterChips : 'flex flex-wrap items-center gap-8px min-w-0'}>
          <FilterChip
            label={allLabel}
            active={selected.length === 0}
            onClick={() => clearDimension(dimension)}
            testId={`tag-chip-${dimension}-all`}
            variant={variant}
          />
          {tags.map((tag) => (
            <FilterChip
              key={tag.preset_tag_id}
              label={resolveTagLabel(tag, localeKey)}
              active={selected.includes(tag.preset_tag_id)}
              onClick={() => toggle(dimension, tag.preset_tag_id)}
              testId={`tag-chip-${dimension}-${tag.key}`}
              variant={variant}
            />
          ))}
        </div>
      </div>
    );
  };

  const renderTagMenu = (dimension: PresetTagDimension, tags: PresetTag[]) => (
    <DropdownMenuSurface>
      <Menu
        onClickMenuItem={(key) =>
          key === 'all' ? clearDimension(dimension) : toggle(dimension, String(key) as PresetTagId)
        }
        className={`min-w-180px max-h-248px overflow-y-auto ${COMPACT_MENU_CLASS}`}
      >
        <Menu.Item key='all'>
          <div className='flex items-center justify-between gap-20px'>
            <span>{allLabel}</span>
            {value[dimension].length === 0 && <Check theme='outline' size={14} className='text-primary-6' />}
          </div>
        </Menu.Item>
        {tags.map((tag) => {
          const active = value[dimension].includes(tag.preset_tag_id);
          return (
            <Menu.Item key={tag.preset_tag_id}>
              <div className='flex items-center justify-between gap-20px'>
                <span className='min-w-0 truncate'>{resolveTagLabel(tag, localeKey)}</span>
                {active && <Check theme='outline' size={14} className='flex-none text-primary-6' />}
              </div>
            </Menu.Item>
          );
        })}
      </Menu>
    </DropdownMenuSurface>
  );

  const selectedSummary = (dimension: PresetTagDimension): string =>
    value[dimension].length === 0
      ? allLabel
      : t('settings.presetTagSelectedCount', {
          defaultValue: '{{count}} selected',
          count: value[dimension].length,
        });

  const renderSelectedRow = (dimension: PresetTagDimension, rowLabel: string, tags: PresetTag[]) => {
    const selectedTags = tags.filter((tag) => value[dimension].includes(tag.preset_tag_id));
    if (selectedTags.length === 0) return null;

    return (
      <div className='flex min-w-0 items-start gap-8px'>
        <span className='w-72px flex-none pt-3px text-12px leading-18px text-[var(--color-text-3)]'>{rowLabel}：</span>
        <div className='flex min-w-0 flex-1 flex-wrap items-center gap-6px'>
          {selectedTags.map((tag) => (
            <div
              key={tag.preset_tag_id}
              role='button'
              tabIndex={0}
              aria-label={`${rowLabel}：${resolveTagLabel(tag, localeKey)}`}
              onClick={() => toggle(dimension, tag.preset_tag_id)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  toggle(dimension, tag.preset_tag_id);
                }
              }}
              className='inline-flex items-center gap-5px rounded-full bg-[var(--color-fill-2)] px-9px py-3px text-12px leading-16px text-[var(--color-text-2)] cursor-pointer hover:bg-[var(--color-fill-3)] hover:text-[var(--color-text-1)] transition-colors'
            >
              <span>{resolveTagLabel(tag, localeKey)}</span>
              <CloseSmall theme='outline' size={12} className='text-[var(--color-text-3)]' />
            </div>
          ))}
        </div>
      </div>
    );
  };

  const hasAudience = audienceTags.length > 0;
  const hasScenario = scenarioTags.length > 0;
  const hasSelection = value.audience.length > 0 || value.scenario.length > 0;

  if (!isDrawer) {
    return (
      <div
        className={[
          filterBarStyles.toolbarContainer,
          'flex w-full flex-col gap-8px',
          className,
        ].filter(Boolean).join(' ')}
      >
        <div className='flex w-full flex-wrap items-center justify-between gap-8px'>
          <div className='flex min-w-0 flex-wrap items-center gap-6px'>
            {hasAudience && (
              <ToolbarSelect
                label={t('settings.presetTagAudience', { defaultValue: 'Audience' })}
                value={selectedSummary('audience')}
                menu={renderTagMenu('audience', audienceTags)}
              />
            )}
            {hasScenario && (
              <ToolbarSelect
                label={t('settings.presetTagScenario', { defaultValue: 'Skill Scenario' })}
                value={selectedSummary('scenario')}
                menu={renderTagMenu('scenario', scenarioTags)}
              />
            )}
            {manageTagsInlineIcon && !hideManageTags && (
              <Tooltip content={manageTagsLabel} position='top' mini>
                <div
                  role='button'
                  tabIndex={0}
                  aria-label={manageTagsLabel}
                  data-testid='btn-manage-tags'
                  onClick={onManageTags}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' || event.key === ' ') {
                      event.preventDefault();
                      onManageTags();
                    }
                  }}
                  className={[
                    'inline-flex h-34px w-34px box-border flex-none items-center justify-center rounded-9px',
                    'border border-solid border-[var(--color-border-3)] bg-[var(--color-bg-2)]',
                    'text-[var(--color-text-2)] cursor-pointer select-none',
                    'hover:border-[var(--color-border-4)] hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]',
                    'focus-visible:outline-none focus-visible:border-primary-6 transition-colors',
                  ].join(' ')}
                >
                  <SettingTwo theme='outline' size={15} strokeWidth={3} />
                </div>
              </Tooltip>
            )}
            {!hasAudience && !hasScenario && (
              <span className='text-12px text-[var(--color-text-3)]'>
                {t('settings.presetTagEmpty', { defaultValue: 'No tags yet. Create some to organize your presets.' })}
              </span>
            )}
          </div>

          {actions ?? (!hideManageTags && !manageTagsInlineIcon && (
            <div
              role='button'
              tabIndex={0}
              data-testid='btn-manage-tags'
              onClick={onManageTags}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  onManageTags();
                }
              }}
              className={[
                'inline-flex h-34px box-border flex-none items-center gap-6px rounded-full px-12px leading-none',
                'border border-solid border-[var(--color-border-3)] bg-[var(--color-bg-2)]',
                'text-13px font-medium text-[var(--color-text-1)] cursor-pointer select-none',
                'hover:border-[var(--color-border-4)] hover:bg-[var(--color-fill-2)]',
                'focus-visible:outline-none focus-visible:border-primary-6 transition-colors',
              ].join(' ')}
            >
              <SettingTwo theme='outline' size={14} strokeWidth={3} />
              {manageTagsLabel}
            </div>
          ))}
        </div>

        {hasSelection && (
          <div className='flex min-h-38px w-full box-border flex-col gap-5px rounded-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-11px py-7px'>
            {renderSelectedRow(
              'audience',
              t('settings.presetTagAudience', { defaultValue: 'Audience' }),
              audienceTags
            )}
            {renderSelectedRow(
              'scenario',
              t('settings.presetTagScenario', { defaultValue: 'Skill Scenario' }),
              scenarioTags
            )}
          </div>
        )}
      </div>
    );
  }

  return (
    <div
      className={[filterBarStyles.drawerFilterBar, className].filter(Boolean).join(' ')}
    >
      <div className='flex items-start justify-between gap-12px'>
        <div className={filterBarStyles.drawerFilterRows}>
          {renderDrawerRow('audience', t('settings.presetTagAudience', { defaultValue: 'Audience' }), audienceTags)}
          {renderDrawerRow('scenario', t('settings.presetTagScenario', { defaultValue: 'Skill Scenario' }), scenarioTags)}
          {!hasAudience && !hasScenario && (
            <span className={filterBarStyles.drawerEmpty}>
              {t('settings.presetTagEmpty', { defaultValue: 'No tags yet. Create some to organize your presets.' })}
            </span>
          )}
        </div>
        {/* Manage tags — a quiet chip-button anchored to the top-right */}
        {!hideManageTags && (
          <div
            role='button'
            tabIndex={0}
            data-testid='btn-manage-tags'
            onClick={onManageTags}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onManageTags();
              }
            }}
            className={filterBarStyles.drawerManageChip}
          >
            <SettingTwo theme='outline' size={13} strokeWidth={3} />
            {t('settings.presetManageTags', { defaultValue: 'Manage Tags' })}
          </div>
        )}
      </div>
    </div>
  );
};

export default PresetTagFilterBar;
