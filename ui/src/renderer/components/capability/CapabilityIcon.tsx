/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Tooltip } from '@arco-design/web-react';
import classNames from 'classnames';
import React from 'react';

/**
 * Shared capability state colours (spec §8 统一 icon 体系). Arco palette vars
 * (`--gray-4` / `--warning-6` / …) are RGB triplets and must be wrapped in
 * `rgb(var(...))`; `--brand` is a plain hex value used as-is.
 *
 * AutoWorkControl / IdmmControl status dots and every session-list capability
 * icon read from this single map — do not re-inline these values.
 */
export const CAPABILITY_COLORS = {
  off: 'rgb(var(--gray-4))',
  idle: 'rgb(var(--warning-6))',
  armed: 'rgb(var(--warning-6))',
  active: 'rgb(var(--success-6))',
  primary: 'rgb(var(--primary-6))',
  danger: 'rgb(var(--danger-6))',
  brand: 'var(--brand)',
} as const;

export interface CapabilityIconProps {
  /** Icon element rendered with `fill='currentColor'` — the colour is applied on the wrapper span. */
  icon: React.ReactNode;
  /** Wrapper colour, normally one of `CAPABILITY_COLORS`. */
  color: string;
  /** Red top-right badge dot (e.g. unread cron executions) — overlay paradigm from CronJobIndicator. */
  dot?: boolean;
  /** Tooltip label. */
  title: string;
  size?: number;
}

/**
 * Single capability marker: coloured icon + optional red badge dot + tooltip.
 * Capability = a per-session / per-workpath feature (cron, AutoWork, IDMM,
 * knowledge base, …) whose presence and run state are surfaced as a tinted icon.
 */
const CapabilityIcon: React.FC<CapabilityIconProps> = ({ icon, color, dot, title, size = 14 }) => (
  <Tooltip content={title} mini>
    <span
      className='relative inline-flex items-center justify-center shrink-0'
      style={{ color, width: size, height: size, lineHeight: 0 }}
    >
      {icon}
      {dot && (
        <span
          className='absolute rounded-full bg-red-500'
          style={{
            width: Math.max(6, size * 0.4),
            height: Math.max(6, size * 0.4),
            top: -1,
            right: -1,
          }}
        />
      )}
    </span>
  </Tooltip>
);

export interface CapabilityIconItem {
  key: string;
  icon: React.ReactNode;
  color: string;
  dot?: boolean;
  title: string;
}

/** At most this many icons render inline; the rest collapse into a `+N` chip. */
const MAX_VISIBLE = 3;

export interface CapabilityIconClusterProps {
  items: CapabilityIconItem[];
  size?: number;
  className?: string;
}

/**
 * Horizontal capability icon strip for session / workpath rows: renders up to
 * three icons, collapsing the overflow into a `+N` chip whose tooltip lists
 * every capability title (the full set, not just the hidden ones).
 */
export const CapabilityIconCluster: React.FC<CapabilityIconClusterProps> = ({ items, size = 14, className }) => {
  if (items.length === 0) return null;
  const visible = items.slice(0, MAX_VISIBLE);
  const overflow = items.length - visible.length;
  return (
    <span className={classNames('inline-flex items-center gap-4px', className)} style={{ lineHeight: 0 }}>
      {visible.map((item) => (
        <CapabilityIcon key={item.key} icon={item.icon} color={item.color} dot={item.dot} title={item.title} size={size} />
      ))}
      {overflow > 0 && (
        <Tooltip
          mini
          content={
            <div className='flex flex-col gap-2px'>
              {items.map((item) => (
                <div key={item.key}>{item.title}</div>
              ))}
            </div>
          }
        >
          <span className='text-11px text-t-tertiary leading-none cursor-default'>+{overflow}</span>
        </Tooltip>
      )}
    </span>
  );
};

export default CapabilityIcon;
