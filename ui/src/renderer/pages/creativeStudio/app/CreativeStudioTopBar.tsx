/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import InstantHoverTooltip from '@renderer/components/base/InstantHoverTooltip';
import WindowControls from '@renderer/components/layout/WindowControls';
import { isDesktopShell, isMacOS } from '@renderer/utils/platform';
import { ArrowLeft } from '@icon-park/react';
import classNames from 'classnames';
import React from 'react';

import styles from './CreativeStudioTopBar.module.css';

interface CreativeStudioTopBarProps {
  title: string;
  backLabel: string;
  onBack: () => void;
}

/** Product-owned window chrome without workbench navigation or session actions. */
const CreativeStudioTopBar: React.FC<CreativeStudioTopBarProps> = ({ title, backLabel, onBack }) => {
  const desktopRuntime = isDesktopShell();
  const macRuntime = desktopRuntime && isMacOS();
  const showWindowControls = desktopRuntime && !macRuntime;

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
      <div className={styles.leading}>
        <InstantHoverTooltip content={backLabel} position='bottom'>
          <button type='button' className={styles.backButton} onClick={onBack} aria-label={backLabel}>
            <ArrowLeft theme='outline' size={17} fill='currentColor' strokeWidth={2.5} />
            <span>{backLabel}</span>
          </button>
        </InstantHoverTooltip>
      </div>
      <div className={styles.title} data-tauri-drag-region>
        {title}
      </div>
      <div className={styles.trailing}>{showWindowControls && <WindowControls />}</div>
    </header>
  );
};

export default CreativeStudioTopBar;
