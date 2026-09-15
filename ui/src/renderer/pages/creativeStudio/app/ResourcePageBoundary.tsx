import React from 'react';
import { Outlet, useLocation } from 'react-router-dom';
import { resourceSectionForPath } from './resourceRoutes';
import styles from './ResourcePageBoundary.module.css';
/** Layout and themed overlay host only; navigation belongs to the desktop rail. */
const ResourcePageBoundary: React.FC = () => {
  const { pathname } = useLocation();
  return (
    <div className={styles.shell} data-resource-page={resourceSectionForPath(pathname)}>
      <main className={styles.content}><Outlet /></main>
      <div id='resource-page-portal-root' className={styles.portalRoot} />
    </div>
  );
};
export default ResourcePageBoundary;
