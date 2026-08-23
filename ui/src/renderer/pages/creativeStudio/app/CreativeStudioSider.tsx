/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Tooltip } from '@arco-design/web-react';
import {
  AddPicture,
  FileText,
  FolderOpen,
  FullScreen,
  ImageFiles,
  VideoTwo,
} from '@icon-park/react';
import classNames from 'classnames';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation } from 'react-router-dom';

import FlexFullContainer from '@renderer/components/layout/FlexFullContainer';
import { getSiderTooltipProps } from '@renderer/utils/ui/siderTooltip';

import {
  CREATIVE_STUDIO_ASSETS_PATH,
  CREATIVE_STUDIO_CANVASES_PATH,
  CREATIVE_STUDIO_IMAGE_PATH,
  CREATIVE_STUDIO_PROMPTS_PATH,
  CREATIVE_STUDIO_ROOT_PATH,
  CREATIVE_STUDIO_VIDEO_PATH,
  creativeStudioSectionForPath,
  type CreativeStudioSection,
} from './routes';
import { normalizeCreativeStudioCanvasesResumeLocation } from './resumeLocation';

interface CreativeStudioSiderProps {
  collapsed?: boolean;
  tooltipEnabled?: boolean;
  /** Last exact route owned by the My Canvases list/editor/Director section. */
  canvasesResumePath?: string;
  onNavigate(path: string): void;
}

interface NavigationIconProps {
  theme?: string;
  size?: string | number;
  strokeWidth?: number;
  className?: string;
}

interface NavigationItem {
  section: Exclude<CreativeStudioSection, 'canvas' | 'director' | 'workflows'>;
  path: string;
  label: string;
  icon: React.ReactElement<NavigationIconProps>;
}

/** Creative Studio navigation rendered in the same rail contract as Settings. */
const CreativeStudioSider: React.FC<CreativeStudioSiderProps> = ({
  collapsed = false,
  tooltipEnabled = false,
  canvasesResumePath = CREATIVE_STUDIO_CANVASES_PATH,
  onNavigate,
}) => {
  const { t } = useTranslation();
  const { pathname } = useLocation();
  const section = creativeStudioSectionForPath(pathname);
  const activeSection =
    section === 'canvas' || section === 'director' ? 'canvases' : section;
  const safeCanvasesResumePath =
    normalizeCreativeStudioCanvasesResumeLocation(canvasesResumePath) ??
    CREATIVE_STUDIO_CANVASES_PATH;
  const siderTooltipProps = getSiderTooltipProps(tooltipEnabled);

  const navigation = useMemo<NavigationItem[]>(
    () => [
      {
        section: 'home',
        path: CREATIVE_STUDIO_ROOT_PATH,
        label: t('creativeStudio.title'),
        icon: <FolderOpen />,
      },
      {
        section: 'canvases',
        path: safeCanvasesResumePath,
        label: t('creativeStudio.navigation.canvases'),
        icon: <FullScreen />,
      },
      {
        section: 'image',
        path: CREATIVE_STUDIO_IMAGE_PATH,
        label: t('creativeStudio.navigation.image'),
        icon: <AddPicture />,
      },
      {
        section: 'video',
        path: CREATIVE_STUDIO_VIDEO_PATH,
        label: t('creativeStudio.navigation.video'),
        icon: <VideoTwo />,
      },
      {
        section: 'prompts',
        path: CREATIVE_STUDIO_PROMPTS_PATH,
        label: t('creativeStudio.navigation.prompts'),
        icon: <FileText />,
      },
      {
        section: 'assets',
        path: CREATIVE_STUDIO_ASSETS_PATH,
        label: t('creativeStudio.navigation.assets'),
        icon: <ImageFiles />,
      },
    ],
    [safeCanvasesResumePath, t]
  );

  return (
    <div
      className={classNames('h-full settings-sider flex flex-col gap-2px overflow-y-auto overflow-x-hidden', {
        'settings-sider--collapsed': collapsed,
      })}
      aria-label={t('creativeStudio.navigation.label')}
      data-creative-studio-sider
    >
      {navigation.map((item) => {
        const isSelected = item.section === activeSection;
        return (
          <Tooltip key={item.section} {...siderTooltipProps} content={item.label} position='right'>
            <button
              type='button'
              data-creative-studio-navigation={item.section}
              className={classNames(
                'settings-sider__item h-34px rd-8px border-0 bg-transparent flex items-center gap-8px group cursor-pointer relative overflow-hidden shrink-0 conversation-item [&.conversation-item+&.conversation-item]:mt-2px transition-colors',
                collapsed ? 'w-full justify-center px-0' : 'w-full justify-start px-10px',
                {
                  'hover:bg-fill-2': !isSelected,
                  '!bg-primary-1 !text-primary-6': isSelected,
                }
              )}
              aria-current={isSelected ? 'page' : undefined}
              onClick={() => onNavigate(item.path)}
            >
              <span className='size-22px flex items-center justify-center shrink-0 line-height-0'>
                {React.cloneElement(item.icon, {
                  theme: 'outline',
                  size: '16',
                  strokeWidth: 3,
                  className: isSelected
                    ? 'block leading-none text-primary-6'
                    : 'block leading-none text-t-secondary',
                })}
              </span>
              <FlexFullContainer className='h-24px collapsed-hidden'>
                <span
                  className={classNames(
                    'settings-sider__item-label text-nowrap overflow-hidden inline-block w-full text-left text-14px font-[500] lh-24px whitespace-nowrap',
                    isSelected ? 'text-primary-6' : 'text-t-primary'
                  )}
                >
                  {item.label}
                </span>
              </FlexFullContainer>
            </button>
          </Tooltip>
        );
      })}
    </div>
  );
};

export default CreativeStudioSider;
