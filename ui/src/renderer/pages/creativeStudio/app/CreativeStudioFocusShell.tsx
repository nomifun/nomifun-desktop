/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Outlet, useLocation, useNavigate } from 'react-router-dom';

import styles from './CreativeStudioFocusShell.module.css';
import CreativeStudioTopBar from './CreativeStudioTopBar';
import { creativeStudioSectionForPath, WORKBENCH_HOME_PATH } from './routes';

/** Full-screen product boundary that preserves the app runtime without its ordinary sidebar. */
const CreativeStudioFocusShell: React.FC = () => {
  const { t } = useTranslation();
  const location = useLocation();
  const navigate = useNavigate();
  const section = creativeStudioSectionForPath(location.pathname);

  const returnToWorkbench = useCallback(() => {
    void navigate(WORKBENCH_HOME_PATH, { replace: true });
  }, [navigate]);

  return (
    <div
      className={classNames('creative-studio-root', styles.shell)}
      data-creative-studio-focus-shell
      data-creative-studio-section={section ?? 'unknown'}
    >
      <CreativeStudioTopBar
        title={t('creativeStudio.title')}
        backLabel={t('creativeStudio.focus.backToWorkbench')}
        onBack={returnToWorkbench}
      />
      <main className={styles.content} data-creative-studio-route-outlet>
        <Outlet />
      </main>
      <div id='creative-studio-portal-root' className={styles.portalRoot} />
    </div>
  );
};

export default CreativeStudioFocusShell;
