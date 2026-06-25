/**
 * SummonDrawer — Right-side drawer with two modes:
 * - Assistant mode: single-select from filtered assistant list.
 * - Skills mode: multi-select from filtered skill list with auto-inject defaults.
 *
 * Reuses AssistantTagFilterBar, filterAssistantsByTags, filterSkillsByTags,
 * useAssistantTags, AssistantAvatar (via DrawerAssistantCard).
 */
import type { Assistant } from '@/common/types/agent/assistantTypes';
import type { SkillInfo } from '@/renderer/pages/settings/AssistantSettings/types';
import type { TagFilterState } from '@/renderer/pages/settings/AssistantSettings/assistantUtils';
import type { SkillTagFilterState } from '@/renderer/pages/settings/skill/skillFilter';

import { ipcBridge } from '@/common';
import { Drawer, Input } from '@arco-design/web-react';
import { Close, Search } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useAssistantTags } from '@/renderer/hooks/assistant';
import AssistantTagFilterBar from '@/renderer/pages/settings/AssistantSettings/AssistantTagFilterBar';
import { filterAssistantsByTags } from '@/renderer/pages/settings/AssistantSettings/assistantUtils';
import { filterSkillsByTags } from '@/renderer/pages/settings/skill/skillFilter';
import coworkSvg from '@/renderer/assets/icons/cowork.svg';
import DrawerAssistantCard from './DrawerAssistantCard';
import DrawerSkillCard from './DrawerSkillCard';
import styles from '../index.module.css';

// ─── Props ────────────────────────────────────────────────────────────────────

export interface SummonDrawerProps {
  visible: boolean;
  mode: 'assistant' | 'skills';
  onModeChange: (m: 'assistant' | 'skills') => void;
  onClose: () => void;
  assistants: Assistant[];
  localeKey: string;
  // Assistant single-select
  onSelectAssistant: (assistantId: string) => void;
  onFree: () => void;
  // Skills multi-select (controlled by parent GuidPage)
  allSkills: Array<{ name: string; description: string; isAuto: boolean }>;
  enabledSkills: string[];
  disabledBuiltinSkills: string[];
  onToggleSkill: (name: string, isAuto: boolean) => void;
}

// ─── Drawer width (responsive, mirrors AssistantEditDrawer) ───────────────────

function computeDrawerWidth(): number {
  const viewportWidth = window.innerWidth || 1024;
  const targetWidth = Math.max(360, Math.floor(viewportWidth * 0.52));
  return Math.min(1024, targetWidth, Math.max(280, viewportWidth - 24));
}

// Avatar image map for AssistantAvatar (same as AssistantSettings)
const AVATAR_IMAGE_MAP: Record<string, string> = {
  'cowork.svg': coworkSvg,
  '\u{1F6E0}\u{FE0F}': coworkSvg,
};

// ─── Component ────────────────────────────────────────────────────────────────

const SummonDrawer: React.FC<SummonDrawerProps> = ({
  visible,
  mode,
  onModeChange,
  onClose,
  assistants,
  localeKey,
  onSelectAssistant,
  onFree,
  allSkills,
  enabledSkills,
  disabledBuiltinSkills,
  onToggleSkill,
}) => {
  const { t } = useTranslation();
  const { audienceTags, scenarioTags } = useAssistantTags();

  // ── Responsive width ──
  const [drawerWidth, setDrawerWidth] = useState(computeDrawerWidth);
  useEffect(() => {
    const handler = () => setDrawerWidth(computeDrawerWidth());
    window.addEventListener('resize', handler);
    return () => window.removeEventListener('resize', handler);
  }, []);

  // ── Local search + tag filter state ──
  const [query, setQuery] = useState('');
  const [tagFilter, setTagFilter] = useState<TagFilterState>({ audience: [], scenario: [] });

  // Reset local state when drawer opens/mode changes
  useEffect(() => {
    if (visible) {
      setQuery('');
      setTagFilter({ audience: [], scenario: [] });
    }
  }, [visible, mode]);

  // ── Skills data (loaded internally for tag-filterable SkillInfo[]) ──
  const [skillInfos, setSkillInfos] = useState<SkillInfo[]>([]);
  const [builtinAutoNames, setBuiltinAutoNames] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (!visible || mode !== 'skills') return;
    let cancelled = false;
    (async () => {
      try {
        const [skills, autoSkills] = await Promise.all([
          ipcBridge.fs.listAvailableSkills.invoke(),
          ipcBridge.fs.listBuiltinAutoSkills.invoke(),
        ]);
        if (cancelled) return;
        setSkillInfos(skills as SkillInfo[]);
        setBuiltinAutoNames(new Set((autoSkills as Array<{ name: string }>).map((s) => s.name)));
      } catch (e) {
        console.warn('[SummonDrawer] failed to load skills', e);
      }
    })();
    return () => { cancelled = true; };
  }, [visible, mode]);

  // ── Filtered results ──
  const filteredAssistants = useMemo(
    () =>
      // AssistantListItem is a type alias for Assistant — structurally identical
      filterAssistantsByTags(assistants, query, tagFilter, localeKey),
    [assistants, query, tagFilter, localeKey]
  );

  const filteredSkills = useMemo(
    () => filterSkillsByTags(skillInfos, query, tagFilter as SkillTagFilterState),
    [skillInfos, query, tagFilter]
  );

  // ── Skill checked state helpers ──
  const isSkillChecked = useCallback(
    (name: string): boolean => {
      const isAuto = builtinAutoNames.has(name);
      return isAuto ? !disabledBuiltinSkills.includes(name) : enabledSkills.includes(name);
    },
    [builtinAutoNames, disabledBuiltinSkills, enabledSkills]
  );

  // ── Selected skill count (for footer) ──
  const selectedSkillCount = useMemo(() => {
    return allSkills.filter((s) => {
      return s.isAuto ? !disabledBuiltinSkills.includes(s.name) : enabledSkills.includes(s.name);
    }).length;
  }, [allSkills, disabledBuiltinSkills, enabledSkills]);

  // ── Handle assistant select (single-select then close) ──
  const handleSelectAssistant = useCallback(
    (id: string) => {
      onSelectAssistant(id);
      onClose();
    },
    [onSelectAssistant, onClose]
  );

  // ── Render ──
  return (
    <Drawer
      closable={false}
      visible={visible}
      placement="right"
      width={drawerWidth}
      zIndex={1200}
      getPopupContainer={() => document.body}
      autoFocus={false}
      onCancel={onClose}
      footer={null}
      headerStyle={{ display: 'none' }}
      bodyStyle={{ padding: 0, height: '100%' }}
    >
      <div className={styles.drawerSurface}>
        {/* Header: Segmented toggle + close */}
        <div className={styles.drawerTopbar}>
          <div
            className={styles.drawerSegmented}
            role='tablist'
            aria-label={`${t('guid.drawer.assistantTab', { defaultValue: '助手' })} / ${t('guid.drawer.skillsTab', { defaultValue: 'Skills' })}`}
          >
            <button
              type='button'
              role='tab'
              aria-selected={mode === 'assistant'}
              className={[
                styles.drawerSegment,
                mode === 'assistant' ? styles.drawerSegmentActive : '',
              ].filter(Boolean).join(' ')}
              onClick={() => onModeChange('assistant')}
            >
              {t('guid.drawer.assistantTab', { defaultValue: '助手' })}
            </button>
            <button
              type='button'
              role='tab'
              aria-selected={mode === 'skills'}
              className={[
                styles.drawerSegment,
                mode === 'skills' ? styles.drawerSegmentActive : '',
              ].filter(Boolean).join(' ')}
              onClick={() => onModeChange('skills')}
            >
              {t('guid.drawer.skillsTab', { defaultValue: 'Skills' })}
            </button>
          </div>

          {/* Close button */}
          <button
            type='button'
            className={styles.drawerCloseButton}
            onClick={onClose}
            aria-label={t('common.close', { defaultValue: 'Close' })}
          >
            <Close theme='outline' size={16} strokeWidth={3} />
          </button>
        </div>

        {/* Search */}
        <div className={styles.drawerSearchPanel}>
          <Input
            prefix={<Search theme='outline' size={15} />}
            placeholder={
              mode === 'assistant'
                ? t('guid.drawer.searchAssistant', { defaultValue: '搜索助手名称或描述...' })
                : t('guid.drawer.searchSkill', { defaultValue: '搜索 Skill 名称或描述...' })
            }
            value={query}
            onChange={setQuery}
            allowClear
            className={styles.drawerSearchInput}
          />
        </div>

        {/* Tag filter */}
        <div className={styles.drawerFilterPanel}>
          <AssistantTagFilterBar
            audienceTags={audienceTags}
            scenarioTags={scenarioTags}
            value={tagFilter}
            onChange={setTagFilter}
            localeKey={localeKey}
            onManageTags={() => {/* Not in scope for this drawer */}}
            variant='drawer'
            hideManageTags
          />
        </div>

        {/* Result count */}
        <div className={styles.drawerResultMeta}>
          <span>
            <strong>{mode === 'assistant' ? filteredAssistants.length : filteredSkills.length}</strong>
            {' '}
            {mode === 'assistant'
              ? t('guid.drawer.assistantCount', { defaultValue: '位助手' })
              : t('guid.drawer.skillCount', { defaultValue: '个 Skill' })}
          </span>
        </div>

        {/* Card list */}
        <div className={styles.drawerList}>
          {mode === 'assistant'
            ? filteredAssistants.map((a) => (
                <DrawerAssistantCard
                  key={a.id}
                  assistant={a}
                  selected={false}
                  localeKey={localeKey}
                  avatarImageMap={AVATAR_IMAGE_MAP}
                  onSelect={handleSelectAssistant}
                />
              ))
            : filteredSkills.map((skill) => (
                <DrawerSkillCard
                  key={skill.name}
                  skill={skill}
                  checked={isSkillChecked(skill.name)}
                  isAuto={builtinAutoNames.has(skill.name)}
                  onToggle={onToggleSkill}
                />
              ))}
        </div>

        {/* Footer */}
        {mode === 'assistant' ? (
          <div className={styles.drawerFooter}>
            <span className={styles.drawerFooterHint}>
              {t('guid.drawer.assistantHint', { defaultValue: '点卡片「召唤」即让本次会话使用该助手（单选）' })}
            </span>
            <button
              type='button'
              className={styles.drawerGhostButton}
              onClick={() => { onFree(); onClose(); }}
            >
              {t('guid.drawer.keepFree', { defaultValue: '保持自由发挥' })}
            </button>
          </div>
        ) : (
          <div className={styles.drawerFooter}>
            <span className={styles.drawerFooterHint}>
              {t('guid.drawer.selectedCount', { defaultValue: '已选 {{count}} 个 Skill', count: selectedSkillCount })}
            </span>
            <button
              type='button'
              className={styles.drawerPrimaryButton}
              onClick={onClose}
            >
              {t('guid.drawer.applySkills', { defaultValue: '应用到本次会话' })}
            </button>
          </div>
        )}
      </div>
    </Drawer>
  );
};

export default SummonDrawer;
