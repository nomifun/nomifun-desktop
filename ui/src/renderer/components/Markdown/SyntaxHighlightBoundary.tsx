/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';

type SyntaxHighlightBoundaryProps = {
  children: React.ReactNode;
  fallback: React.ReactNode;
  resetKey: string;
};

type SyntaxHighlightBoundaryState = {
  failed: boolean;
};

/** Keep a broken syntax grammar from taking down the entire conversation route. */
class SyntaxHighlightBoundary extends React.Component<
  SyntaxHighlightBoundaryProps,
  SyntaxHighlightBoundaryState
> {
  state: SyntaxHighlightBoundaryState = { failed: false };

  static getDerivedStateFromError(): SyntaxHighlightBoundaryState {
    return { failed: true };
  }

  componentDidUpdate(previousProps: SyntaxHighlightBoundaryProps): void {
    if (this.state.failed && previousProps.resetKey !== this.props.resetKey) {
      this.setState({ failed: false });
    }
  }

  render(): React.ReactNode {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}

export default SyntaxHighlightBoundary;
