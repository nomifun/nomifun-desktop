/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React from 'react';
import { Outlet, useLocation } from 'react-router-dom';

import styles from './CreativeStudioFocusShell.module.css';
import { creativeStudioSectionForPath } from './routes';

/** Product route boundary hosted inside the application's shared titlebar and sidebar layout. */
const CreativeStudioFocusShell: React.FC = () => {
  const location = useLocation();
  const section = creativeStudioSectionForPath(location.pathname);

  return (
    <div
      className={classNames('creative-studio-root', styles.shell)}
      data-creative-studio-focus-shell
      data-creative-studio-section={section ?? 'unknown'}
    >
      <main className={styles.content} data-creative-studio-route-outlet>
        <Outlet />
      </main>
      <div id='creative-studio-portal-root' className={styles.portalRoot} />
    </div>
  );
};

export default CreativeStudioFocusShell;
