/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Outlet, useNavigate } from 'react-router-dom';

import CreativeStudioTopBar from './CreativeStudioTopBar';
import styles from './CreativeStudioFocusShell.module.css';
import { WORKBENCH_HOME_PATH } from './routes';

/**
 * Full-screen product boundary for the rebuilt Creative Studio.
 *
 * The authenticated application runtime and global theme stay mounted above
 * this shell, while the normal workbench sidebar, shortcuts and pull-to-refresh
 * layout are deliberately absent.
 */
const CreativeStudioFocusShell: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const returnToWorkbench = useCallback(() => {
    void navigate(WORKBENCH_HOME_PATH, { replace: true });
  }, [navigate]);

  return (
    <div className={classNames('creative-studio-root', styles.shell)} data-creative-studio-focus-shell>
      <CreativeStudioTopBar
        title={t('creativeStudio.title')}
        backLabel={t('creativeStudio.focus.backToWorkbench')}
        onBack={returnToWorkbench}
      />
      <main className={styles.content}>
        <Outlet />
      </main>
      <div id='creative-studio-portal-root' className={styles.portalRoot} />
    </div>
  );
};

export default CreativeStudioFocusShell;
