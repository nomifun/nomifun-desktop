/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import ImageWorkbenchComposer from './ImageWorkbenchComposer';
import ImageWorkbenchResults from './ImageWorkbenchResults';
import type { ImageWorkbenchProps } from './types';
import styles from './ImageWorkbench.module.css';

/**
 * Controlled visual workbench. It owns no model catalog, upload, generation,
 * persistence or deletion side effects; adapters provide those through props.
 */
const ImageWorkbench: React.FC<ImageWorkbenchProps> = (props) => (
  <div
    className={styles.workbench}
    data-image-workbench
    data-workbench-layout={props.layout}
    data-task-state={props.task.state}
  >
    <main className={props.layout === 'side' ? styles.sideLayout : styles.bottomLayout}>
      {props.layout === 'side' ? <ImageWorkbenchComposer {...props} /> : null}
      <ImageWorkbenchResults
        results={props.results}
        selectedResultIds={props.selectedResultIds}
        task={props.task}
        onSelectionChange={props.onResultSelectionChange}
        onDeleteResult={props.onDeleteResult}
        onDeleteSelected={props.onDeleteSelected}
        onRetryResult={props.onRetryResult}
        onLoadResult={props.onLoadResult}
        onCancelTask={props.onCancelTask}
        historyLoading={props.historyLoading}
        historyError={props.historyError}
        historyLoadingMore={props.historyLoadingMore}
        historyHasMore={props.historyHasMore}
        onLoadMoreResults={props.onLoadMoreResults}
      />
      {props.layout === 'bottom' ? <ImageWorkbenchComposer {...props} /> : null}
    </main>
  </div>
);

export default ImageWorkbench;
