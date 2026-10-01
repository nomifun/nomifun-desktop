/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useMemo, useSyncExternalStore } from 'react';

interface ComposeIssue {
  nodeId: string;
  message: string;
}

interface ComposeRequestSnapshot<Submission> {
  busyNodes: ReadonlySet<string>;
  issues: ReadonlyMap<string, ComposeIssue>;
  submissions: ReadonlyMap<string, Submission>;
}

/** Submission admission and retry state belong to the authored node, not a media type. */
export function useCanvasComposeRequests<Submission extends { nodeId: string }>(scopeKey: string) {
  const store = useMemo(() => {
    const empty = (): ComposeRequestSnapshot<Submission> => ({
      busyNodes: new Set<string>(),
      issues: new Map<string, ComposeIssue>(),
      submissions: new Map<string, Submission>(),
    });
    let snapshot = empty();
    const listeners = new Set<() => void>();
    const publish = (next: typeof snapshot) => {
      snapshot = next;
      for (const listener of listeners) listener();
    };
    return {
      getSnapshot: () => snapshot,
      subscribe: (listener: () => void) => {
        listeners.add(listener);
        return () => { listeners.delete(listener); };
      },
      isBusy: (nodeId: string) => snapshot.busyNodes.has(nodeId),
      setBusy: (busy: boolean, nodeId: string) => {
        const busyNodes = new Set(snapshot.busyNodes);
        if (busy) busyNodes.add(nodeId);
        else busyNodes.delete(nodeId);
        publish({ ...snapshot, busyNodes });
      },
      setIssue: (issue: ComposeIssue | null, nodeId = issue?.nodeId) => {
        if (!nodeId) return;
        const issues = new Map(snapshot.issues);
        if (issue) issues.set(nodeId, issue);
        else issues.delete(nodeId);
        publish({ ...snapshot, issues });
      },
      setSubmission: (submission: Submission | null, nodeId = submission?.nodeId) => {
        if (!nodeId) return;
        const submissions = new Map(snapshot.submissions);
        if (submission) submissions.set(nodeId, submission);
        else submissions.delete(nodeId);
        publish({ ...snapshot, submissions });
      },
      reset: () => publish(empty()),
    };
  }, [scopeKey]);
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  return { ...store, ...snapshot, busy: snapshot.busyNodes.size > 0 };
}
