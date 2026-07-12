# Workspace Tool Rail Red-Dot Indicator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the right-side Changes icon's numeric badge with a compact red dot when pending workspace changes exist.

**Architecture:** Keep `changeCount` as the source of truth in `WorkspaceToolRail`; convert it to a visibility condition for an empty indicator element. Scope the visual change to the existing tool-rail badge CSS, preserving the tool rail's layout and all panel behavior.

**Tech Stack:** React 19, TypeScript, CSS, Bun test runner, TypeScript compiler.

## Global Constraints

- Change only the desktop workspace tool rail's Changes indicator.
- Show a red dot only when `changeCount > 0`; hide it for zero or missing values.
- Do not expose numeric count text in the visual badge.
- Preserve icon size, workspace panel interactions, mobile controls, and status dots.

---

### Task 1: Convert the Changes badge into a red-dot indicator

**Files:**
- Modify: `ui/src/renderer/pages/conversation/components/ChatLayout/WorkspaceToolRail.tsx:111-116`
- Modify: `ui/src/renderer/pages/conversation/components/ChatLayout/chat-layout.css:147-164`
- Test: `ui/src/renderer/pages/conversation/components/ChatLayout/workspaceToolRail.test.ts`

**Interfaces:**
- Consumes: existing optional `changeCount?: number` prop.
- Produces: a decorative `.workspace-tool-rail__badge` element only when `changeCount > 0`.

- [ ] **Step 1: Write the failing test**

Add assertions that lock in a text-free, compact red-dot indicator:

```ts
test('uses a text-free red dot when workspace changes are pending', () => {
  const badge = rule('\\.workspace-tool-rail__badge');

  expect(componentSource).toContain("changeCount > 0 ? <span className='workspace-tool-rail__badge' /> : undefined");
  expect(componentSource).not.toContain("changeCount > 99 ? '99+' : changeCount");
  expect(badge.includes('width: 7px;')).toBe(true);
  expect(badge.includes('height: 7px;')).toBe(true);
  expect(badge.includes('background: var(--color-danger-light-4);')).toBe(true);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bun test ui/src/renderer/pages/conversation/components/ChatLayout/workspaceToolRail.test.ts`

Expected: FAIL because the component still interpolates the numeric count and the badge CSS is a 15px primary-coloured pill.

- [ ] **Step 3: Write minimal implementation**

In `WorkspaceToolRail.tsx`, replace the numeric badge content with an empty indicator:

```tsx
badge={changeCount > 0 ? <span className='workspace-tool-rail__badge' /> : undefined}
```

In `chat-layout.css`, replace the pill dimensions and text styles with:

```css
.workspace-tool-rail__badge {
  position: absolute;
  top: 3px;
  right: 3px;
  width: 7px;
  height: 7px;
  border: 1px solid var(--bg-1);
  border-radius: 50%;
  background: var(--color-danger-light-4);
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun test ui/src/renderer/pages/conversation/components/ChatLayout/workspaceToolRail.test.ts`

Expected: PASS with the tool-rail structural suite green.

- [ ] **Step 5: Run typecheck**

Run: `bun --cwd ui run typecheck`

Expected: exits 0 with no TypeScript diagnostics.

- [ ] **Step 6: Review and commit**

Run: `git diff --check && git diff -- ui/src/renderer/pages/conversation/components/ChatLayout/WorkspaceToolRail.tsx ui/src/renderer/pages/conversation/components/ChatLayout/chat-layout.css ui/src/renderer/pages/conversation/components/ChatLayout/workspaceToolRail.test.ts`

Then commit using `git add ui/src/renderer/pages/conversation/components/ChatLayout/WorkspaceToolRail.tsx ui/src/renderer/pages/conversation/components/ChatLayout/chat-layout.css ui/src/renderer/pages/conversation/components/ChatLayout/workspaceToolRail.test.ts` followed by `git commit -m "fix(ui): use red dot for workspace changes"`.
