/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Spin } from '@arco-design/web-react';
import { Brain, Right } from '@icon-park/react';
import classNames from 'classnames';
import React, { useEffect, useRef, useState } from 'react';

import styles from './ThinkingProcessDisplay.module.css';

export type ThinkingProcessDisplayState = 'running' | 'completed';
export type ThinkingProcessDisplayVariant = 'standalone' | 'process';

export interface ThinkingProcessDisplayProps {
  state: ThinkingProcessDisplayState;
  subject?: string;
  content?: string;
  startedAt?: number;
  /** Stable identity used to reset local elapsed/expansion state between rows. */
  identityKey?: string;
  variant?: ThinkingProcessDisplayVariant;
  /** Header-only mode for runtimes that expose activity but no thinking body. */
  disclosure?: boolean;
  expanded?: boolean;
  onExpandedChange?: (expanded: boolean) => void;
  runningFallbackLabel?: string;
  completedLabel?: string;
  formatElapsedTime?: (seconds: number) => string;
  className?: string;
  role?: React.AriaRole;
}

const defaultFormatElapsedTime = (seconds: number): string => {
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${seconds % 60}s`;
};

/**
 * Shared presentation for desktop thinking rows and header-only Agent activity.
 * Transport-specific events stay outside this component.
 */
const ThinkingProcessDisplay: React.FC<ThinkingProcessDisplayProps> = ({
  state,
  subject = '',
  content = '',
  startedAt,
  identityKey,
  variant = 'standalone',
  disclosure = true,
  expanded,
  onExpandedChange,
  runningFallbackLabel = 'Thinking...',
  completedLabel = 'Thought complete',
  formatElapsedTime = defaultFormatElapsedTime,
  className,
  role,
}) => {
  const isDone = state === 'completed';
  const isProcessVariant = variant === 'process';
  const defaultExpanded = expanded ?? (isProcessVariant ? !isDone : true);
  const [internalExpanded, setInternalExpanded] = useState(() => defaultExpanded);
  const resolvedExpanded = expanded ?? internalExpanded;
  const [elapsedTime, setElapsedTime] = useState(() => {
    const initialStartedAt = startedAt ?? Date.now();
    return isDone ? 0 : Math.max(0, Math.floor((Date.now() - initialStartedAt) / 1000));
  });
  const startTimeRef = useRef<number>(startedAt ?? Date.now());
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (expanded !== undefined) return;
    setInternalExpanded(defaultExpanded);
  }, [defaultExpanded, expanded, identityKey]);

  useEffect(() => {
    if (isDone) return;

    startTimeRef.current = startedAt ?? Date.now();
    setElapsedTime(Math.max(0, Math.floor((Date.now() - startTimeRef.current) / 1000)));
    const timer = setInterval(() => {
      setElapsedTime(Math.floor((Date.now() - startTimeRef.current) / 1000));
    }, 1000);

    return () => clearInterval(timer);
  }, [identityKey, isDone, startedAt]);

  useEffect(() => {
    if (disclosure && !isDone && resolvedExpanded && bodyRef.current) {
      bodyRef.current.scrollTop = bodyRef.current.scrollHeight;
    }
  }, [content, disclosure, isDone, resolvedExpanded]);

  const handleToggle = () => {
    if (!disclosure) return;
    const nextExpanded = !resolvedExpanded;
    if (expanded === undefined) {
      setInternalExpanded(nextExpanded);
    }
    onExpandedChange?.(nextExpanded);
  };

  const summaryText = isDone
    ? completedLabel
    : `${subject.trim() || runningFallbackLabel} · ${formatElapsedTime(elapsedTime)}`;

  return (
    <div
      className={classNames(
        styles.container,
        isProcessVariant && styles.containerProcess,
        className
      )}
      data-thinking-process-state={state}
      data-thinking-process-disclosure={disclosure}
      role={role}
    >
      <div
        className={classNames(
          styles.header,
          isProcessVariant && styles.headerProcess,
          !disclosure && styles.headerStatic
        )}
        data-thinking-process-header
        onClick={disclosure ? handleToggle : undefined}
      >
        <span className={styles.headerIcon}>
          {!isDone ? <Spin size={12} /> : <Brain theme='outline' size='14' />}
        </span>
        <span className={classNames(styles.summary, !disclosure && styles.summaryStatic)}>
          {summaryText}
        </span>
        {disclosure ? (
          <span
            className={classNames(styles.arrow, resolvedExpanded && styles.arrowExpanded)}
            data-thinking-process-toggle
          >
            <Right theme='outline' size='12' />
          </span>
        ) : null}
      </div>
      {disclosure ? (
        <div
          ref={bodyRef}
          className={classNames(
            styles.body,
            isProcessVariant && styles.bodyProcess,
            !resolvedExpanded && styles.collapsed
          )}
          data-thinking-process-body
        >
          {content}
        </div>
      ) : null}
    </div>
  );
};

export default ThinkingProcessDisplay;
