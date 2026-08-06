/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Input } from '@arco-design/web-react';
import type { InputProps, RefInputType } from '@arco-design/web-react/es/Input';
import classNames from 'classnames';
import React from 'react';

export interface NomiInputProps extends InputProps {
  /** Shrink the field to its visible value while keeping practical bounds. */
  contentFit?: boolean;
  contentMinWidth?: React.CSSProperties['minWidth'];
  contentMaxWidth?: React.CSSProperties['maxWidth'];
}

const NomiInput = React.forwardRef<RefInputType, NomiInputProps>(
  (
    {
      className,
      contentFit = false,
      contentMinWidth = 0,
      contentMaxWidth = 320,
      autoWidth,
      style,
      ...rest
    },
    ref
  ) => (
    <Input
      ref={ref}
      className={classNames('nomi-input', contentFit && 'shrink-0', className)}
      autoWidth={contentFit ? { minWidth: contentMinWidth, maxWidth: contentMaxWidth } : autoWidth}
      style={{ ...(contentFit ? { flex: 'none' } : undefined), ...style }}
      {...rest}
    />
  )
);

NomiInput.displayName = 'NomiInput';

export default NomiInput;
