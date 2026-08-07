/**
 * PresetListPanel — Renders presets as a responsive card grid with a
 * two-dimension tag filter bar (Audience / Skill Scenario) and compact actions.
 * Replaces the old source-Tabs + enabled/disabled-section layout.
 */
import { filterPresetsByTags, type TagFilterState } from './presetUtils';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import type { PresetReference, PresetTag } from '@/common/types/agent/presetTypes';
import type { PresetListItem } from './types';
import PresetCard from './PresetCard';
import PresetTagFilterBar from './PresetTagFilterBar';
import toolbarStyles from './PresetTagFilterBar.module.css';
import { Tooltip } from '@arco-design/web-react';
import { AddOne, Search, SettingTwo } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

/**
 * 卡片网格按「内容容器实际宽度」自动定列(auto-fill),而非视口断点 —— 设置内容
 * 面板被一级 rail + 二级 ContentSider 占去宽度,视口宽 ≠ 面板可用宽。设定卡片
 * 比 AgentCard 更厚(头像+名称+描述+标签+开关),取较宽的 232px 下限。
 * Card grids auto-fit columns to the actual container width (not viewport
 * breakpoints): the settings pane is narrower than the viewport, so viewport
 * breakpoints over-column and clip cards on a narrow pane.
 */
const CARD_GRID_COLS = 'repeat(auto-fill, minmax(min(232px, 100%), 1fr))';

type PresetListPanelProps = {
  presets: PresetListItem[];
  localeKey: string;
  avatarImageMap: Record<string, string>;
  isExtensionPreset: (preset: PresetListItem | null | undefined) => boolean;
  onEdit: (preset: PresetListItem) => void;
  onDuplicate: (preset: PresetListItem) => void;
  onCreate: () => void;
  onToggleEnabled: (preset: PresetListItem, checked: boolean) => void;
  setActivePresetId: (id: PresetReference) => void;
  /** When set, scroll to and highlight the matching preset card */
  highlightId?: string | null;
  /** Called after the highlight animation completes so the parent can clear the param */
  onHighlightConsumed?: () => void;
  // Tag facets
  audienceTags: PresetTag[];
  scenarioTags: PresetTag[];
  tagById: Map<string, PresetTag>;
  onManageTags: () => void;
};

const PresetListPanel: React.FC<PresetListPanelProps> = ({
  presets,
  localeKey,
  avatarImageMap,
  isExtensionPreset,
  onEdit,
  onDuplicate,
  onCreate,
  onToggleEnabled,
  setActivePresetId,
  highlightId,
  onHighlightConsumed,
  audienceTags,
  scenarioTags,
  tagById,
  onManageTags,
}) => {
  const { t } = useTranslation();
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;
  const [search_query, setSearchQuery] = useState('');
  const [tagFilter, setTagFilter] = useState<TagFilterState>({ audience: [], scenario: [] });
  const [highlightedId, setHighlightedId] = useState<string | null>(null);
  const cardRefs = useRef<Record<string, HTMLDivElement | null>>({});
  const cardRefSetter = useCallback(
    (id: string) => (el: HTMLDivElement | null) => {
      cardRefs.current[id] = el;
    },
    []
  );

  // Scroll to and highlight an preset card when navigated with ?highlight=id.
  // Depends on `presets` so it re-runs after async data loads and refs are
  // populated. A short delay ensures the layout is settled on first mount.
  useEffect(() => {
    if (!highlightId || presets.length === 0) return;
    const el = cardRefs.current[highlightId];
    if (!el) return;

    const timer = setTimeout(() => {
      el.scrollIntoView({ behavior: 'smooth', block: 'center' });
      setHighlightedId(highlightId);
      setTimeout(() => {
        setHighlightedId(null);
        onHighlightConsumed?.();
      }, 2000);
    }, 150);

    return () => clearTimeout(timer);
  }, [highlightId, presets, onHighlightConsumed]);

  // Self-heal the tag filter against the current vocabulary: when a tag is
  // deleted in the management modal, its chip vanishes from the bar but a
  // selected key could linger in `tagFilter.<dim>`, invisibly constraining the
  // facet. Drop any selected key that no longer exists. The `return prev`
  // no-change guard prevents render loops.
  useEffect(() => {
    const audIds = new Set<string>(audienceTags.map((tag) => tag.preset_tag_id));
    const scnIds = new Set<string>(scenarioTags.map((tag) => tag.preset_tag_id));
    setTagFilter((prev) => {
      const audience = prev.audience.filter((id) => audIds.has(id));
      const scenario = prev.scenario.filter((id) => scnIds.has(id));
      if (audience.length === prev.audience.length && scenario.length === prev.scenario.length) return prev;
      return { audience, scenario };
    });
  }, [audienceTags, scenarioTags]);

  const filteredPresets = useMemo(
    () => filterPresetsByTags(presets, search_query, tagFilter, localeKey),
    [presets, search_query, tagFilter, localeKey]
  );

  const searchLabel = t('settings.searchPresets', { defaultValue: 'Search presets...' });
  const manageTagsLabel = t('settings.presetManageTags', { defaultValue: 'Manage Tags' });
  const createPresetLabel = t('settings.createPreset', { defaultValue: 'Create Preset' });

  return (
    <div>
      <div
        data-testid='preset-library-surface'
        className={`mt-8px box-border rounded-24px border border-solid border-[var(--color-border-2)] bg-transparent ${isMobile ? 'px-16px py-10px' : 'px-20px py-12px'}`}
      >
        <div className='flex flex-col gap-10px mb-12px'>
          <div className='min-w-0'>
            <p className='m-0 max-w-[760px] text-14px text-t-secondary leading-relaxed'>
              {t('settings.presetsListDescription', {
                defaultValue:
                  'Save Agent instructions, preferences, Skills and knowledge scope as reusable one-click configurations.',
              })}
            </p>
          </div>

          <PresetTagFilterBar
            audienceTags={audienceTags}
            scenarioTags={scenarioTags}
            value={tagFilter}
            onChange={setTagFilter}
            localeKey={localeKey}
            onManageTags={onManageTags}
            actions={(
              <div
                className={[
                  'flex min-w-0 items-center gap-6px',
                  isMobile ? 'w-full' : 'flex-1 justify-end',
                  !isMobile ? toolbarStyles.desktopActions : '',
                ].join(' ')}
              >
                <Tooltip content={searchLabel} position='top' mini>
                  <div
                    className={[
                      'flex h-34px box-border min-w-0 items-center gap-7px rounded-full px-11px',
                      'border border-solid border-[var(--color-border-3)] bg-[var(--color-bg-2)]',
                      'focus-within:border-primary-6 transition-colors',
                      isMobile ? 'flex-1' : 'w-220px',
                      !isMobile ? toolbarStyles.desktopSearch : '',
                    ].join(' ')}
                  >
                    <span
                      className={[
                        'inline-flex h-18px w-18px flex-none items-center justify-center',
                        toolbarStyles.actionIcon,
                        !isMobile ? toolbarStyles.desktopSearchIcon : '',
                      ].join(' ')}
                    >
                      <Search theme='outline' size={14} className='block text-[var(--color-text-3)]' />
                    </span>
                    <input
                      aria-label={searchLabel}
                      data-testid='input-search-preset'
                      className={[
                        'w-full border-none bg-transparent text-13px leading-18px text-[var(--color-text-1)] outline-none font-[inherit] placeholder:text-[var(--color-text-3)]',
                        !isMobile ? toolbarStyles.desktopSearchInput : '',
                      ].join(' ')}
                      placeholder={searchLabel}
                      value={search_query}
                      onChange={(event) => setSearchQuery(event.target.value)}
                    />
                  </div>
                </Tooltip>

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
                      'inline-flex h-34px box-border flex-none items-center gap-6px rounded-full px-12px leading-none',
                      'border border-solid border-[var(--color-border-3)] bg-[var(--color-bg-2)]',
                      'text-13px font-medium text-[var(--color-text-1)] cursor-pointer select-none',
                      'hover:border-[var(--color-border-4)] hover:bg-[var(--color-fill-2)]',
                      'focus-visible:outline-none focus-visible:border-primary-6 transition-colors',
                      !isMobile ? toolbarStyles.desktopIconAction : '',
                    ].join(' ')}
                  >
                    <span className={`${toolbarStyles.actionIcon} inline-flex h-18px w-18px flex-none items-center justify-center`}>
                      <SettingTwo theme='outline' size={14} strokeWidth={3} className='block' />
                    </span>
                    {!isMobile && (
                      <span className={`${toolbarStyles.desktopActionLabel} inline-flex h-18px items-center leading-18px`}>
                        {manageTagsLabel}
                      </span>
                    )}
                  </div>
                </Tooltip>

                <Tooltip content={createPresetLabel} position='top' mini>
                  <div
                    role='button'
                    tabIndex={0}
                    aria-label={createPresetLabel}
                    data-testid='btn-create-preset'
                    onClick={onCreate}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter' || event.key === ' ') {
                        event.preventDefault();
                        onCreate();
                      }
                    }}
                    className={[
                      'inline-flex h-34px box-border flex-none items-center gap-6px cursor-pointer select-none leading-none',
                      'rounded-full px-14px text-13px font-700',
                      'border border-solid border-transparent',
                      'bg-[rgba(var(--primary-6),0.12)] text-[var(--color-text-1)]',
                      'hover:bg-[rgba(var(--primary-6),0.18)]',
                      'focus-visible:border-primary-6 focus-visible:outline-none transition-colors',
                      !isMobile ? toolbarStyles.desktopIconAction : '',
                    ].join(' ')}
                  >
                    <span className={`${toolbarStyles.actionIcon} inline-flex h-18px w-18px flex-none items-center justify-center`}>
                      <AddOne theme='outline' size={15} strokeWidth={4} className='block text-primary-6' />
                    </span>
                    {!isMobile && (
                      <span className={`${toolbarStyles.desktopActionLabel} inline-flex h-18px items-center leading-18px`}>
                        {createPresetLabel}
                      </span>
                    )}
                  </div>
                </Tooltip>
              </div>
            )}
          />
        </div>

        {filteredPresets.length > 0 ? (
          <div className='grid gap-12px' style={{ gridTemplateColumns: CARD_GRID_COLS }}>
            {filteredPresets.map((preset) => (
              <PresetCard
                key={preset.preset_id}
                preset={preset}
                localeKey={localeKey}
                avatarImageMap={avatarImageMap}
                tagById={tagById}
                isExtensionPreset={isExtensionPreset}
                onEdit={(a) => {
                  setActivePresetId(a.preset_id);
                  onEdit(a);
                }}
                onDuplicate={onDuplicate}
                onToggleEnabled={onToggleEnabled}
                highlighted={highlightedId === preset.preset_id}
                cardRef={cardRefSetter(preset.preset_id)}
              />
            ))}
          </div>
        ) : (
          <div className='text-center text-t-secondary py-32px'>
            {presets.length === 0
              ? t('settings.presetsEmpty', {
                  defaultValue: 'No presets yet. Create one to save a reusable launch configuration.',
                })
              : t('settings.presetNoMatch', {
                  defaultValue: 'No presets match the current filters.',
                })}
          </div>
        )}
      </div>
    </div>
  );
};

export default PresetListPanel;
