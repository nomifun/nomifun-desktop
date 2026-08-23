/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Navigate, useSearchParams } from 'react-router-dom';
import classNames from 'classnames';
import {
  BroadcastRadio,
  Comment,
  HeadsetOne,
  LinkCloud,
  SettingTwo,
  Lightning,
  Pic,
  PreviewOpen,
  SafeRetrieval,
  VideoTwo,
  Voice,
} from '@icon-park/react';
import ContentSider from '@/renderer/components/layout/ContentSider';
import SegmentedTabs, { type SegmentedTabItem } from '@/renderer/components/base/SegmentedTabs';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import { useResizableSplit } from '@/renderer/hooks/ui/useResizableSplit';
import { useContainerWidth } from '@/renderer/hooks/ui/useContainerWidth';
import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';
import ModelModalContent from '@/renderer/components/settings/SettingsModal/contents/ModelModalContent';
import ModelFailoverContent from './ModelFailoverContent';
import FreeModelsContent from './FreeModelsContent';
import SpeechToTextContent from './SpeechToTextContent';
import TextToSpeechContent from './TextToSpeechContent';
import ChatModelsContent from './ChatModelsContent';
import RealtimeModelsContent from './RealtimeModelsContent';
import VisionModelsContent from './VisionModelsContent';
import ImageModelsContent from './ImageModelsContent';
import ImageEditModelsContent from './ImageEditModelsContent';
import VideoModelsContent from './VideoModelsContent';
import EmbeddingModelsContent from './EmbeddingModelsContent';
import RerankModelsContent from './RerankModelsContent';

type Section =
  | 'models'
  | 'chat'
  | 'realtime'
  | 'asr'
  | 'tts'
  | 'vision'
  | 'image'
  | 'image-edit'
  | 'video'
  | 'embedding'
  | 'rerank'
  | 'free'
  | 'failover';

/**
 * Retired section keys kept resolvable so old bookmarks and links land somewhere
 * sensible. `speech` and `creation` were hosts that each held several model
 * categories; they now have one section per category, so an old link resolves to
 * the first of them.
 */
const LEGACY_SECTIONS: Record<string, Section> = {
  speech: 'asr',
  creation: 'image',
  // 「全局模型设置」曾是 IDMM 全局默认 + 故障转移队列 + 决策活动的三 tab 宿主。
  // 全局 IDMM 那套已整体删除,剩下的只有故障转移队列,所以这一栏就叫它自己。
  global: 'failover',
};

const SECTION_KEYS: readonly Section[] = [
  'models',
  'chat',
  'realtime',
  'asr',
  'tts',
  'vision',
  'image',
  'image-edit',
  'video',
  'embedding',
  'rerank',
  'free',
  'failover',
];

const isSection = (value: string | null): value is Section =>
  value !== null && (SECTION_KEYS as readonly string[]).includes(value);

/** Resolve a `?section=` value, following the retired-key aliases. */
const resolveSection = (value: string | null): Section | null =>
  isSection(value) ? value : value !== null && value in LEGACY_SECTIONS ? LEGACY_SECTIONS[value] : null;

const MODELHUB_SIDER_STORAGE_KEY = 'nomifun:modelhub-sider-width';

interface SectionDef {
  key: Section;
  labelKey: I18nKey;
  icon: React.ReactNode;
}

interface SectionGroup {
  key: string;
  titleKey: I18nKey;
  sections: SectionDef[];
}

/**
 * The sidebar's three groups, in the order a model actually travels: a provider
 * is the source of every model, so 供应商与密钥 leads; then one section per model
 * capability; then the things you reach for rarely. 免费模型 sits in the last
 * group on purpose — it is NomiFun-managed, not something the user configured,
 * and the same rule orders the provider groups inside every capability section.
 */
const SECTION_GROUPS: SectionGroup[] = [
  {
    key: 'access',
    titleKey: 'settings.modelHub.groupAccess',
    sections: [
      {
        key: 'models',
        labelKey: 'settings.modelHub.sectionModels',
        icon: <LinkCloud theme='outline' size='16' strokeWidth={3} />,
      },
    ],
  },
  {
    key: 'capability',
    titleKey: 'settings.modelHub.groupCapability',
    sections: [
      {
        key: 'chat',
        labelKey: 'settings.modelHub.sectionChat',
        icon: <Comment theme='outline' size='16' strokeWidth={3} />,
      },
      {
        key: 'realtime',
        labelKey: 'settings.modelHub.sectionRealtime',
        icon: <BroadcastRadio theme='outline' size='16' strokeWidth={3} />,
      },
      {
        key: 'asr',
        labelKey: 'settings.modelHub.sectionAsr',
        icon: <HeadsetOne theme='outline' size='16' strokeWidth={3} />,
      },
      {
        key: 'tts',
        labelKey: 'settings.modelHub.sectionTts',
        icon: <Voice theme='outline' size='16' strokeWidth={3} />,
      },
      {
        key: 'vision',
        labelKey: 'settings.modelHub.sectionVision',
        icon: <PreviewOpen theme='outline' size='16' strokeWidth={3} />,
      },
      {
        key: 'image',
        labelKey: 'settings.modelHub.sectionImage',
        icon: <Pic theme='outline' size='16' strokeWidth={3} />,
      },
      {
        key: 'image-edit',
        labelKey: 'settings.modelHub.sectionImageEdit',
        icon: <Pic theme='outline' size='16' strokeWidth={3} />,
      },
      {
        key: 'video',
        labelKey: 'settings.modelHub.sectionVideo',
        icon: <VideoTwo theme='outline' size='16' strokeWidth={3} />,
      },
      {
        key: 'embedding',
        labelKey: 'settings.modelHub.sectionEmbedding',
        icon: <SafeRetrieval theme='outline' size='16' strokeWidth={3} />,
      },
      {
        key: 'rerank',
        labelKey: 'settings.modelHub.sectionRerank',
        icon: <SafeRetrieval theme='outline' size='16' strokeWidth={3} />,
      },
    ],
  },
  {
    key: 'advanced',
    titleKey: 'settings.modelHub.groupAdvanced',
    sections: [
      {
        key: 'free',
        labelKey: 'settings.modelHub.sectionFree',
        icon: <Lightning theme='outline' size='16' strokeWidth={3} />,
      },
      {
        key: 'failover',
        labelKey: 'settings.modelHub.sectionFailover',
        icon: <SettingTwo theme='outline' size='16' strokeWidth={3} />,
      },
    ],
  },
];

const FLAT_SECTIONS: SectionDef[] = SECTION_GROUPS.flatMap((group) => group.sections);

/**
 * ModelHubPage (/models) — "Model Management", a CAPABILITY-first view. The
 * primary level is a content-area secondary sidebar (mirroring the conversation
 * `ContentSider`): a grouped left section list drives the right content pane.
 * Execution engines live independently under Settings and are intentionally not
 * mixed into model management.
 *
 * One sidebar entry = one model capability, so nothing hides behind a page-level
 * filter or a second row of tabs. The sidebar width is drag-resizable and
 * persisted. On mobile the sidebar collapses to a horizontal segmented bar above
 * the content (flat — the groups are a desktop affordance).
 *
 * The level syncs to `?section=`; the retired keys (`speech`, `creation`,
 * `global`) still resolve so old bookmarks work.
 */
const ModelHubPage: React.FC = () => {
  const { t } = useTranslation();
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;
  const [searchParams, setSearchParams] = useSearchParams();

  const [section, setSection] = useState<Section>(
    () => resolveSection(searchParams.get('section')) ?? 'chat'
  );

  useEffect(() => {
    const resolved = resolveSection(searchParams.get('section'));
    if (resolved && resolved !== section) {
      setSection(resolved);
    }
  }, [searchParams, section]);

  const handleSectionChange = useCallback(
    (key: string) => {
      if (!isSection(key)) return;
      setSection(key);
      const next = new URLSearchParams(searchParams);
      next.set('section', key);
      setSearchParams(next, { replace: true });
    },
    [searchParams, setSearchParams]
  );

  const focusSectionTab = useCallback((key: Section) => {
    requestAnimationFrame(() => {
      document.getElementById(`model-hub-tab-${key}`)?.focus();
    });
  }, []);

  const resize = useResizableSplit({
    unit: 'px',
    defaultWidth: 248,
    minWidth: 200,
    maxWidth: 360,
    storageKey: MODELHUB_SIDER_STORAGE_KEY,
  });

  // 内容面板的可用宽度 = 视口 − 一次 rail − 二级 ContentSider − 拖拽宽度，远小于视口。
  // 按面板实宽（而非视口断点）给横向 padding，窄面板不再被 md:px-40px 白吃 80px。
  const { ref: paneRef, width: paneWidth } = useContainerWidth<HTMLDivElement>();
  const panePadX = paneWidth === 0 ? 'px-24px' : paneWidth >= 600 ? 'px-40px' : paneWidth >= 420 ? 'px-24px' : 'px-16px';

  const content = (
    <>
      {section === 'models' && <ModelModalContent />}
      {section === 'chat' && <ChatModelsContent />}
      {section === 'realtime' && <RealtimeModelsContent />}
      {section === 'asr' && <SpeechToTextContent />}
      {section === 'tts' && <TextToSpeechContent />}
      {section === 'vision' && <VisionModelsContent />}
      {section === 'image' && <ImageModelsContent />}
      {section === 'image-edit' && <ImageEditModelsContent />}
      {section === 'video' && <VideoModelsContent />}
      {section === 'embedding' && <EmbeddingModelsContent />}
      {section === 'rerank' && <RerankModelsContent />}
      {section === 'free' && <FreeModelsContent />}
      {section === 'failover' && <ModelFailoverContent />}
    </>
  );

  // Compatibility for bookmarks and links from builds where execution engines
  // were embedded in model management. The engine page has a single surface
  // now, so every legacy sub-tab resolves to it.
  if (searchParams.get('section') === 'agents') {
    return <Navigate to='/settings/execution-engines' replace />;
  }

  // Mobile: horizontal segmented nav above the content (no left sidebar).
  if (isMobile) {
    const segmentedItems: SegmentedTabItem[] = FLAT_SECTIONS.map((s) => ({
      key: s.key,
      label: t(s.labelKey),
      icon: s.icon,
    }));
    return (
      <div className='w-full min-h-full box-border overflow-y-auto px-16px py-16px'>
        <div className='text-20px font-600 text-t-primary leading-tight'>{t('settings.modelHub.title')}</div>
        <div className='mt-4px mb-14px text-12px leading-18px text-t-secondary'>{t('settings.modelHub.subtitle')}</div>
        <div className='mb-16px'>
          <SegmentedTabs items={segmentedItems} activeKey={section} onChange={handleSectionChange} size='sm' />
        </div>
        {content}
      </div>
    );
  }

  const siderHeader = (
    <div className='px-16px pt-16px pb-10px'>
      <div className='text-15px font-600 text-t-primary leading-none'>{t('settings.modelHub.title')}</div>
      <div className='mt-4px text-12px leading-18px text-t-secondary'>{t('settings.modelHub.subtitle')}</div>
    </div>
  );

  const renderTab = (s: SectionDef) => {
    const selected = section === s.key;
    const index = FLAT_SECTIONS.findIndex((item) => item.key === s.key);
    return (
      <div
        key={s.key}
        id={`model-hub-tab-${s.key}`}
        role='tab'
        aria-selected={selected}
        aria-controls='model-hub-panel'
        tabIndex={selected ? 0 : -1}
        onClick={() => handleSectionChange(s.key)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            handleSectionChange(s.key);
            return;
          }
          if (e.key === 'ArrowUp' || e.key === 'ArrowDown' || e.key === 'Home' || e.key === 'End') {
            e.preventDefault();
            const nextIndex =
              e.key === 'Home'
                ? 0
                : e.key === 'End'
                  ? FLAT_SECTIONS.length - 1
                  : (index + (e.key === 'ArrowDown' ? 1 : -1) + FLAT_SECTIONS.length) % FLAT_SECTIONS.length;
            const next = FLAT_SECTIONS[nextIndex].key;
            handleSectionChange(next);
            focusSectionTab(next);
          }
        }}
        className={classNames(
          'h-34px rd-8px flex items-center gap-8px px-10px cursor-pointer shrink-0 transition-colors outline-none text-t-primary',
          selected ? '!bg-primary-1 !text-primary-6' : 'hover:bg-fill-2 active:bg-fill-3'
        )}
      >
        <span
          className={classNames(
            'size-22px flex items-center justify-center shrink-0 line-height-0',
            selected ? 'text-primary-6' : 'text-t-secondary'
          )}
        >
          {s.icon}
        </span>
        <span className='text-14px font-[500] leading-24px truncate'>{t(s.labelKey)}</span>
      </div>
    );
  };

  return (
    <div className='relative flex size-full min-h-0'>
      <ContentSider
        width={resize.splitRatio}
        header={siderHeader}
        ariaLabel={t('settings.modelHub.title')}
        resizeHandle={resize.createDragHandle({ className: 'right-0' })}
      >
        {/* The group captions are `aria-hidden` decoration: a `tablist` may own
            only `tab` children, so exposing them would break that contract while
            the tabs themselves already carry their labels and position. */}
        <div className='flex flex-col px-8px pb-8px' role='tablist' aria-orientation='vertical'>
          {SECTION_GROUPS.map((group, groupIndex) => (
            <React.Fragment key={group.key}>
              <div
                aria-hidden='true'
                className={classNames(
                  'px-10px pb-4px text-11px font-600 leading-16px text-t-tertiary select-none',
                  groupIndex === 0 ? 'pt-2px' : 'pt-12px'
                )}
              >
                {t(group.titleKey)}
              </div>
              <div className='flex flex-col gap-2px'>{group.sections.map(renderTab)}</div>
            </React.Fragment>
          ))}
        </div>
      </ContentSider>
      <div
        id='model-hub-panel'
        className='flex-1 min-w-0 min-h-0 overflow-y-auto'
        role='tabpanel'
        aria-labelledby={`model-hub-tab-${section}`}
        ref={paneRef}
      >
        <div className={classNames('mx-auto w-full max-w-1100px box-border py-32px', panePadX)}>{content}</div>
      </div>
    </div>
  );
};

export default ModelHubPage;
