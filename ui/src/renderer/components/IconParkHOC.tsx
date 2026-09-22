/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

// Runtime-imported by the IconPark transform in ui/vite.config.ts. Source-only
// reachability scanners cannot see the generated import, so keep the matching
// repository guard in scripts/check-icon-imports.mjs.
import React from 'react';
import { IconProvider, DEFAULT_ICON_CONFIGS } from '@icon-park/react/es/runtime';
import { iconColors } from '@/renderer/styles/colors';

const DEFAULT_ICON_SIZE = 16;

const IconParkHOC = <T extends Record<string, any>>(Component: React.FunctionComponent<T>): React.FC<T> => {
  return (props) => {
    return React.createElement(
      IconProvider,
      {
        value: {
          ...DEFAULT_ICON_CONFIGS,
          size: DEFAULT_ICON_SIZE,
        },
      },
      [
        React.createElement(Component, {
          key: 'c3',
          strokeWidth: 3,
          fill: iconColors.secondary,
          ...props,
          className: 'cursor-pointer  ' + ((props as any).className || ''),
        }),
      ]
    );
  };
};

export default IconParkHOC;
