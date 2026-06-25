/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';

interface RouteErrorBoundaryProps {
  children: React.ReactNode;
}

interface RouteErrorBoundaryState {
  error: Error | null;
  componentStack: string | null;
}

/**
 * RouteErrorBoundary — 路由级错误边界
 *
 * The app previously had NO error boundary, so any render/throw inside a route
 * blanked the entire window (white screen) with no visible cause. This boundary
 * wraps each lazily-loaded route element (via `withRouteFallback`) so a crash in
 * one page renders a readable error panel — message + stack + React component
 * stack — instead of taking down the whole shell. The surrounding app chrome
 * (titlebar, primary sidebar) stays alive, and the error text is selectable so
 * it can be copied for diagnosis.
 *
 * Each route gets its own boundary instance (the element is remounted on
 * navigation), so moving to another route clears the error automatically.
 */
class RouteErrorBoundary extends React.Component<RouteErrorBoundaryProps, RouteErrorBoundaryState> {
  state: RouteErrorBoundaryState = { error: null, componentStack: null };

  static getDerivedStateFromError(error: Error): Partial<RouteErrorBoundaryState> {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo): void {
    // Surface to the console too (devtools, if available) — keep the on-screen
    // panel as the primary channel since release builds may not expose devtools.
    // eslint-disable-next-line no-console
    console.error('[RouteErrorBoundary] route crashed:', error, info.componentStack);
    this.setState({ componentStack: info.componentStack ?? null });
  }

  private handleReset = (): void => {
    this.setState({ error: null, componentStack: null });
  };

  render(): React.ReactNode {
    const { error, componentStack } = this.state;
    if (!error) return this.props.children;

    return (
      <div
        role='alert'
        style={{
          height: '100%',
          width: '100%',
          overflow: 'auto',
          padding: '24px',
          boxSizing: 'border-box',
          background: '#1b1115',
          color: '#ffd9d9',
          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
          fontSize: '13px',
          lineHeight: 1.55,
        }}
      >
        <div style={{ fontSize: '15px', fontWeight: 700, color: '#ff6b6b', marginBottom: '12px' }}>
          页面渲染出错（已被路由错误边界捕获，未影响其它页面）
        </div>
        <div style={{ fontWeight: 700, marginBottom: '8px', userSelect: 'text' }}>
          {error.name}: {error.message}
        </div>
        <button
          type='button'
          onClick={this.handleReset}
          style={{
            marginBottom: '16px',
            padding: '4px 12px',
            border: '1px solid #ff6b6b',
            borderRadius: '6px',
            background: 'transparent',
            color: '#ffd9d9',
            cursor: 'pointer',
          }}
        >
          重试
        </button>
        {error.stack ? (
          <>
            <div style={{ opacity: 0.7, marginBottom: '4px' }}>Stack</div>
            <pre style={{ whiteSpace: 'pre-wrap', userSelect: 'text', margin: '0 0 16px' }}>{error.stack}</pre>
          </>
        ) : null}
        {componentStack ? (
          <>
            <div style={{ opacity: 0.7, marginBottom: '4px' }}>Component stack</div>
            <pre style={{ whiteSpace: 'pre-wrap', userSelect: 'text', margin: 0 }}>{componentStack}</pre>
          </>
        ) : null}
      </div>
    );
  }
}

export default RouteErrorBoundary;
