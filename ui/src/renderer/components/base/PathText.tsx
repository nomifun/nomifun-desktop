/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React from 'react';

import { splitPath } from '@/renderer/utils/file/pathDisplay';

type PathTextProps = {
  /** Absolute filesystem path to render. */
  path: string;
  /** Class applied to the outer container (font size / weight / color). */
  className?: string;
};

/**
 * Renders a filesystem path with middle truncation: the parent directory
 * collapses behind an ellipsis while the final segment stays fully visible, so
 * same-named folders under different parents remain distinguishable in tight
 * widths (the sidebar, the workspace pill). Pure CSS — the head shrinks with
 * `min-w-0`, the tail is `shrink-0` — so no width measurement is needed.
 */
const PathText: React.FC<PathTextProps> = ({ path, className }) => {
  const { head, tail } = splitPath(path);
  if (!head) {
    return <span className={classNames('truncate', className)}>{tail || path}</span>;
  }
  return (
    <span className={classNames('flex items-center min-w-0 overflow-hidden', className)}>
      <span className='overflow-hidden text-ellipsis whitespace-nowrap'>{head}</span>
      <span className='shrink-0 whitespace-nowrap'>{tail}</span>
    </span>
  );
};

export default PathText;
