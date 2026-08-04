/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React from 'react';

interface NomiSettingSectionProps {
  title: React.ReactNode;
  description?: React.ReactNode;
  action?: React.ReactNode;
  titleClassName?: string;
  className?: string;
  children: React.ReactNode;
}

export const NomiSettingSection = React.forwardRef<HTMLElement, NomiSettingSectionProps>(
  ({ title, description, action, titleClassName, className, children }, ref) => (
    <section ref={ref} className={classNames('flex flex-col gap-8px', className)}>
      <div className='flex min-w-0 items-end justify-between gap-12px'>
        <div className='min-w-0 flex-1'>
          <h2 className={classNames('m-0 text-14px leading-20px font-600 text-t-primary', titleClassName)}>
            {title}
          </h2>
          {description && <p className='m-0 mt-3px text-12px leading-18px text-t-tertiary'>{description}</p>}
        </div>
        {action && <div className='flex shrink-0 items-center justify-end'>{action}</div>}
      </div>
      {children}
    </section>
  )
);

NomiSettingSection.displayName = 'NomiSettingSection';

interface NomiSettingListProps {
  className?: string;
  children: React.ReactNode;
}

export const NomiSettingList: React.FC<NomiSettingListProps> = ({ className, children }) => (
  <div
    className={classNames(
      'overflow-hidden rd-10px border border-solid border-[var(--color-border-2)] bg-transparent',
      className
    )}
  >
    {React.Children.toArray(children).map((child, index) => (
      <div
        key={(child as React.ReactElement).key ?? index}
        className={index === 0 ? undefined : 'border-t border-t-solid border-t-[var(--color-border-2)]'}
      >
        {child}
      </div>
    ))}
  </div>
);

interface NomiSettingRowProps {
  title: React.ReactNode;
  description?: React.ReactNode;
  controls?: React.ReactNode;
  footer?: React.ReactNode;
  leading?: React.ReactNode;
  className?: string;
  titleClassName?: string;
  descriptionClassName?: string;
  controlsClassName?: string;
  style?: React.CSSProperties;
}

export const NomiSettingRow: React.FC<NomiSettingRowProps> = ({
  title,
  description,
  controls,
  footer,
  leading,
  className,
  titleClassName,
  descriptionClassName,
  controlsClassName,
  style,
}) => (
  <div className={classNames('bg-[var(--color-bg-2)] px-10px py-6px', className)} style={style}>
    <div className='flex min-h-28px min-w-0 items-center gap-12px max-[760px]:flex-col max-[760px]:items-stretch'>
      <div className='min-w-0 flex-1'>
        <div className={classNames('flex min-w-0 items-center gap-6px text-14px font-500 text-t-primary', titleClassName)}>
          {leading}
          <div className='min-w-0'>{title}</div>
        </div>
        {description && (
          <div className={classNames('mt-2px text-12px leading-18px text-t-tertiary', descriptionClassName)}>
            {description}
          </div>
        )}
      </div>
      {controls && (
        <div
          className={classNames(
            'flex max-w-[62%] shrink-0 items-center justify-end gap-8px flex-wrap max-[760px]:max-w-full max-[760px]:justify-start',
            controlsClassName
          )}
        >
          {controls}
        </div>
      )}
    </div>
    {footer && (
      <div className='mt-8px border-t border-t-solid border-t-[var(--color-border-2)] pt-8px'>{footer}</div>
    )}
  </div>
);
