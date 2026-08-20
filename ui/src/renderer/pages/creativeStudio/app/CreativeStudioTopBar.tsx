/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import InstantHoverTooltip from '@renderer/components/base/InstantHoverTooltip';
import WindowControls from '@renderer/components/layout/WindowControls';
import { isDesktopShell, isMacOS } from '@renderer/utils/platform';
import {
  AddPicture,
  ArrowLeft,
  FileText,
  FolderOpen,
  FullScreen,
  ImageFiles,
  Moon,
  SunOne,
  VideoTwo,
} from '@icon-park/react';
import classNames from 'classnames';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { Link, useLocation } from 'react-router-dom';

import styles from './CreativeStudioTopBar.module.css';
import {
  CREATIVE_STUDIO_ASSETS_PATH,
  CREATIVE_STUDIO_IMAGE_PATH,
  CREATIVE_STUDIO_PROJECTS_PATH,
  CREATIVE_STUDIO_PROMPTS_PATH,
  CREATIVE_STUDIO_VIDEO_PATH,
  creativeStudioSectionForPath,
  type CreativeStudioSection,
} from './routes';

export interface CreativeStudioTopBarProps {
  title: string;
  backLabel: string;
  theme: 'light' | 'dark';
  onToggleTheme: () => void;
  onBack: () => void;
}

interface NavigationItem {
  section: Exclude<CreativeStudioSection, 'canvas' | 'audio'>;
  path: string;
  label: string;
  icon: React.ReactNode;
}

/** Source product navigation; audio remains a route, but is not a source top-level destination. */
const CreativeStudioTopBar: React.FC<CreativeStudioTopBarProps> = ({
  title,
  backLabel,
  theme,
  onToggleTheme,
  onBack,
}) => {
  const { t } = useTranslation();
  const location = useLocation();
  const desktopRuntime = isDesktopShell();
  const macRuntime = desktopRuntime && isMacOS();
  const showWindowControls = desktopRuntime && !macRuntime;
  const section = creativeStudioSectionForPath(location.pathname);
  const activeSection = section === 'canvas' ? 'projects' : section;
  const themeToggleLabel = theme === 'light'
    ? t('settings.darkMode', { defaultValue: '深色' })
    : t('settings.lightMode', { defaultValue: '浅色' });
  const navigation: NavigationItem[] = [
    {
      section: 'projects',
      path: CREATIVE_STUDIO_PROJECTS_PATH,
      label: t('creativeStudio.navigation.projects', { defaultValue: '我的画布' }),
      icon: <FullScreen theme='outline' size={16} fill='currentColor' strokeWidth={3} />,
    },
    {
      section: 'image',
      path: CREATIVE_STUDIO_IMAGE_PATH,
      label: t('creativeStudio.navigation.image', { defaultValue: '生图工作台' }),
      icon: <AddPicture theme='outline' size={16} fill='currentColor' strokeWidth={3} />,
    },
    {
      section: 'video',
      path: CREATIVE_STUDIO_VIDEO_PATH,
      label: t('creativeStudio.navigation.video', { defaultValue: '视频创作台' }),
      icon: <VideoTwo theme='outline' size={16} fill='currentColor' strokeWidth={3} />,
    },
    {
      section: 'prompts',
      path: CREATIVE_STUDIO_PROMPTS_PATH,
      label: t('creativeStudio.navigation.prompts', { defaultValue: '提示词库' }),
      icon: <FileText theme='outline' size={16} fill='currentColor' strokeWidth={3} />,
    },
    {
      section: 'assets',
      path: CREATIVE_STUDIO_ASSETS_PATH,
      label: t('creativeStudio.navigation.assets', { defaultValue: '我的素材' }),
      icon: <ImageFiles theme='outline' size={16} fill='currentColor' strokeWidth={3} />,
    },
  ];

  const handleDoubleClick = (event: React.MouseEvent<HTMLElement>) => {
    if (!desktopRuntime || macRuntime) return;
    const target = event.target as HTMLElement | null;
    if (!target?.hasAttribute('data-tauri-drag-region')) return;
    void ipcBridge.windowControls.toggleMaximize.invoke();
  };

  return (
    <header
      className={classNames(styles.topBar, {
        [styles.desktop]: desktopRuntime,
        [styles.mac]: macRuntime,
      })}
      data-creative-studio-top-bar
      data-tauri-drag-region
      onDoubleClick={handleDoubleClick}
    >
      <div className={styles.inner} data-tauri-drag-region>
        <Link
          to={CREATIVE_STUDIO_PROJECTS_PATH}
          className={styles.brand}
          aria-label={title}
          data-creative-studio-brand
        >
          <FolderOpen theme='outline' size={20} fill='currentColor' strokeWidth={3} />
          <span>{title}</span>
        </Link>

        <nav
          className={styles.navigation}
          aria-label={t('creativeStudio.navigation.label', { defaultValue: '创意工坊' })}
        >
          {navigation.map((item) => {
            const active = item.section === activeSection;
            return (
              <Link
                key={item.section}
                to={item.path}
                className={styles.navigationItem}
                data-active={active || undefined}
                data-creative-studio-navigation={item.section}
                aria-current={active ? 'page' : undefined}
              >
                {item.icon}
                <span>{item.label}</span>
              </Link>
            );
          })}
        </nav>

        <div className={styles.trailing}>
          <InstantHoverTooltip content={themeToggleLabel} position='bottom'>
            <button
              type='button'
              className={styles.iconButton}
              onClick={onToggleTheme}
              aria-label={themeToggleLabel}
              aria-pressed={theme === 'dark'}
            >
              {theme === 'light' ? (
                <Moon theme='outline' size={17} fill='currentColor' strokeWidth={3} />
              ) : (
                <SunOne theme='outline' size={17} fill='currentColor' strokeWidth={3} />
              )}
            </button>
          </InstantHoverTooltip>
          <InstantHoverTooltip content={backLabel} position='bottom'>
            <button
              type='button'
              className={styles.backButton}
              onClick={onBack}
              aria-label={backLabel}
            >
              <ArrowLeft theme='outline' size={16} fill='currentColor' strokeWidth={3} />
              <span>{backLabel}</span>
            </button>
          </InstantHoverTooltip>
          {showWindowControls && <WindowControls />}
        </div>
      </div>
    </header>
  );
};

export default CreativeStudioTopBar;
