/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Spin } from '@arco-design/web-react';
import { Brain, Right } from '@icon-park/react';
import type { ThinkingContentDisplayLength } from '@/common/config/thinkingDisplay';
import classNames from 'classnames';
import React, { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';

import styles from './ThinkingProcessDisplay.module.css';

export type ThinkingProcessDisplayState = 'running' | 'completed';
export type ThinkingProcessDisplayVariant = 'standalone' | 'process';

export interface ThinkingProcessDisplayProps {
  state: ThinkingProcessDisplayState;
  subject?: string;
  content?: string;
  /** Optional formatted body; content still identifies streaming updates. */
  children?: React.ReactNode;
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
  completedSummary?: string;
  bodyLength?: ThinkingContentDisplayLength;
  /** Disable the row clock when its enclosing turn already shows total time. */
  showElapsedTime?: boolean;
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
  children,
  startedAt,
  identityKey,
  variant = 'standalone',
  disclosure = true,
  expanded,
  onExpandedChange,
  runningFallbackLabel = 'Thinking...',
  completedLabel = 'Thought complete',
  completedSummary = '',
  bodyLength = 'full',
  showElapsedTime = true,
  formatElapsedTime = defaultFormatElapsedTime,
  className,
  role,
}) => {
  const isDone = state === 'completed';
  const isProcessVariant = variant === 'process';
  const defaultExpanded = expanded ?? !isDone;
  const [internalExpanded, setInternalExpanded] = useState(() => defaultExpanded);
  const resolvedExpanded = expanded ?? internalExpanded;
  const [elapsedTime, setElapsedTime] = useState(() => {
    const initialStartedAt = startedAt ?? Date.now();
    return isDone || !showElapsedTime ? 0 : Math.max(0, Math.floor((Date.now() - initialStartedAt) / 1000));
  });
  const startTimeRef = useRef<number>(startedAt ?? Date.now());
  const bodyRef = useRef<HTMLDivElement>(null);
  const bodyId = useId();
  const Header = disclosure ? 'button' : 'div';

  // A phase transition owns the automatic disclosure default. Repeated body
  // updates within that phase keep the user's manual choice.
  useLayoutEffect(() => {
    if (expanded !== undefined) return;
    setInternalExpanded(defaultExpanded);
  }, [defaultExpanded, expanded, identityKey]);

  useEffect(() => {
    if (isDone || !showElapsedTime) return;

    startTimeRef.current = startedAt ?? Date.now();
    setElapsedTime(Math.max(0, Math.floor((Date.now() - startTimeRef.current) / 1000)));
    const timer = setInterval(() => {
      setElapsedTime(Math.floor((Date.now() - startTimeRef.current) / 1000));
    }, 1000);

    return () => clearInterval(timer);
  }, [identityKey, isDone, showElapsedTime, startedAt]);

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

  const runningLabel = subject.trim() || runningFallbackLabel;
  const summaryText = isDone
    ? [completedLabel, completedSummary].filter(Boolean).join(' · ')
    : showElapsedTime ? `${runningLabel} · ${formatElapsedTime(elapsedTime)}` : runningLabel;

  return (
    <div
      className={classNames(
        styles.container,
        isProcessVariant && styles.containerProcess,
        className
      )}
      data-thinking-process-state={state}
      data-thinking-process-identity={identityKey}
      data-thinking-process-disclosure={disclosure}
      data-thinking-body-length={bodyLength}
      role={role}
    >
      <Header
        type={disclosure ? 'button' : undefined}
        className={classNames(
          styles.header,
          isProcessVariant && styles.headerProcess,
          !disclosure && styles.headerStatic
        )}
        data-thinking-process-header
        onClick={disclosure ? handleToggle : undefined}
        aria-expanded={disclosure ? resolvedExpanded : undefined}
        aria-controls={disclosure ? bodyId : undefined}
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
      </Header>
      {disclosure ? (
        <div
          ref={bodyRef}
          id={bodyId}
          className={classNames(
            styles.body,
            children != null && styles.bodyRich,
            isProcessVariant && styles.bodyProcess,
            bodyLength !== 'full' && styles.bodyLimited,
            bodyLength === 'compact' && styles.bodyCompact,
            !resolvedExpanded && styles.collapsed
          )}
          data-thinking-process-body
        >
          {children ?? content}
        </div>
      ) : null}
    </div>
  );
};

export default ThinkingProcessDisplay;
