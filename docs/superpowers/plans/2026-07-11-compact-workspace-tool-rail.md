# Compact Workspace Tool Rail Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the desktop conversation workspace tool rail 32px wide without changing the 48px height of its tool entries.

**Architecture:** This is a CSS-only desktop layout adjustment. A focused Bun source-contract test reads the stylesheet and locks the rail, item, and mobile-trigger dimensions so future styling changes cannot accidentally re-expand the rail or alter the requested entry height.

**Tech Stack:** React, CSS, Bun test runner, TypeScript.

## Global Constraints

- Only desktop `.workspace-tool-rail` styles may change; mobile trigger behavior stays unchanged.
- `.workspace-tool-rail` width, flex basis, and min-width must each be `32px`.
- `.workspace-tool-rail__item` width must be `28px` and height must remain `48px`.
- Keep the icon, label, hover, active, focus, badge, status, divider, and collapse controls.

---

### Task 1: Lock and apply compact desktop rail dimensions

**Files:**
- Create: `ui/src/renderer/pages/conversation/components/ChatLayout/workspaceToolRail.test.ts`
- Modify: `ui/src/renderer/pages/conversation/components/ChatLayout/chat-layout.css:65-126`

**Interfaces:**
- Consumes: CSS selectors `.workspace-tool-rail`, `.workspace-tool-rail__item`, and `.workspace-tool-rail-mobile-trigger`.
- Produces: A 32px desktop rail with 28px-wide, 48px-high tool entries; an executable source-contract test.

- [x] **Step 1: Write the failing CSS contract test**

```ts
import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const stylesheet = readFileSync(new URL('./chat-layout.css', import.meta.url), 'utf8');

const rule = (selector: string) => {
  const match = stylesheet.match(new RegExp(`${selector}\\s*\\{([\\s\\S]*?)\\n\\}`, 'm'));
  expect(match).not.toBeNull();
  return match?.[1] ?? '';
};

describe('workspace tool rail dimensions', () => {
  test('uses the compact desktop width while preserving control height', () => {
    const rail = rule('\\.workspace-tool-rail');
    const item = rule('\\.workspace-tool-rail__item');

    expect(rail.includes('flex: 0 0 32px;')).toBe(true);
    expect(rail.includes('width: 32px;')).toBe(true);
    expect(rail.includes('min-width: 32px;')).toBe(true);
    expect(item.includes('width: 28px;')).toBe(true);
    expect(item.includes('height: 48px;')).toBe(true);
  });

  test('does not change the mobile workspace trigger dimensions', () => {
    const trigger = rule('\\.workspace-tool-rail-mobile-trigger');

    expect(trigger.includes('width: 24px;')).toBe(true);
    expect(trigger.includes('height: 70px;')).toBe(true);
  });
});
```

- [x] **Step 2: Run the test and verify it fails against the 54px rail**

Run: `bun test ui/src/renderer/pages/conversation/components/ChatLayout/workspaceToolRail.test.ts`

Expected: FAIL because the stylesheet contains `54px` rail sizing and `42px` item width.

- [x] **Step 3: Make the minimal style change**

```css
.workspace-tool-rail {
  flex: 0 0 32px;
  width: 32px;
  min-width: 32px;
  gap: 4px;
  padding: 10px 2px;
}

.workspace-tool-rail__item {
  width: 28px;
  height: 48px;
}
```

Leave the label, icon, active, hover, focus, badge, status, divider, footer, collapse, and mobile-trigger selectors intact.

- [x] **Step 4: Run the targeted test and verify it passes**

Run: `bun test ui/src/renderer/pages/conversation/components/ChatLayout/workspaceToolRail.test.ts`

Expected: PASS with both compact-desktop and unchanged-mobile assertions passing.

- [x] **Step 5: Run the front-end type check**

Run: `bun run typecheck`

Expected: exit code 0 with no TypeScript diagnostics.

- [ ] **Step 6: Commit the implementation**

```bash
git add ui/src/renderer/pages/conversation/components/ChatLayout/chat-layout.css \
  ui/src/renderer/pages/conversation/components/ChatLayout/workspaceToolRail.test.ts \
  docs/superpowers/plans/2026-07-11-compact-workspace-tool-rail.md
git commit -m "style: compact workspace tool rail"
```
