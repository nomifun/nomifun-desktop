# Requirements Checkbox Selection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the requirements-list selection controls a fully filled dark selected state with a centered, inset white checkmark and smooth accessible animation.

**Architecture:** Keep Arco Checkbox semantics and the global theme control contract intact. Mark the two requirements-only controls with one scoped class, then add the scoped mask and indicator rules to the runtime-injected theme control contract so custom preset CSS cannot override them. A source-contract test protects the scope and key visual-state rules.

**Tech Stack:** React 19, TypeScript, Arco Design Checkbox, CSS custom properties, Bun tests.

## Global Constraints

- Change only the requirement row and filter-toolbar checkboxes.
- Use `--control-selected-bg` and `--control-selected-fg` for theme-aware selected colors.
- Preserve global keyboard focus, indeterminate, and disabled contracts.
- Use a 160 ms ease-out transition and disable it with `prefers-reduced-motion`.

---

### Task 1: Add the scoped checkbox treatment

**Files:**
- Create: `ui/src/renderer/pages/requirements/WorkspacePage/RequirementCheckboxSelection.test.ts`
- Modify: `ui/src/renderer/pages/requirements/WorkspacePage/RequirementListRow.tsx:84`
- Modify: `ui/src/renderer/pages/requirements/WorkspacePage/RequirementFilters.tsx:303-311`
- Modify: `ui/src/renderer/styles/theme-control-contract.css`

**Interfaces:**
- Consumes: Arco `.arco-checkbox-mask`, `.arco-checkbox-mask-icon`, checked, and indeterminate class structure.
- Produces: `.requirements-selection-checkbox`, used only by the two requirements controls.

- [ ] **Step 1: Write the failing test**

Create a Bun test which reads the two TSX sources and stylesheet. Assert both sources contain `className='requirements-selection-checkbox'`, CSS has `.requirements-selection-checkbox.arco-checkbox-checked .arco-checkbox-mask`, the white indicator rule, and `@media (prefers-reduced-motion: reduce)`.

- [ ] **Step 2: Run test to verify it fails**

Run `bun test src/renderer/pages/requirements/WorkspacePage/RequirementCheckboxSelection.test.ts` from `ui/`. Expected: FAIL because the class and stylesheet do not exist.

- [ ] **Step 3: Write minimal implementation**

Add the scoped class to both Arco checkbox instances. Add CSS to the runtime-injected theme control contract that clips the mask, fills it with `--control-selected-bg` when checked or indeterminate, centers and insets the existing indicator, and transitions mask and indicator properties for 160 ms ease-out. Preserve disabled styles and disable transitions for reduced motion.

- [ ] **Step 4: Run test to verify it passes**

Run `bun test src/renderer/pages/requirements/WorkspacePage/RequirementCheckboxSelection.test.ts` from `ui/`. Expected: PASS with zero failures.

- [ ] **Step 5: Commit**

Run `git add ui/src/renderer/pages/requirements/WorkspacePage/RequirementCheckboxSelection.test.ts ui/src/renderer/pages/requirements/WorkspacePage/RequirementListRow.tsx ui/src/renderer/pages/requirements/WorkspacePage/RequirementFilters.tsx ui/src/renderer/styles/theme-control-contract.css` then `git commit -m "fix(ui): refine requirement checkbox selection"`.

### Task 2: Verify requirements and theme regressions

**Files:**
- Test: `ui/src/renderer/pages/requirements/WorkspacePage/RequirementCheckboxSelection.test.ts`
- Test: `ui/src/renderer/pages/requirements/WorkspacePage/RequirementFilters.test.tsx`
- Test: `ui/src/renderer/styles/themeControlContract.test.ts`

**Interfaces:**
- Consumes: The scoped class and stylesheet from Task 1.
- Produces: Fresh evidence that requirements behavior, global theme controls, and TypeScript remain valid.

- [ ] **Step 1: Run focused test suite**

Run `bun test src/renderer/pages/requirements/WorkspacePage/RequirementCheckboxSelection.test.ts src/renderer/pages/requirements/WorkspacePage/RequirementFilters.test.tsx src/renderer/styles/themeControlContract.test.ts` from `ui/`. Expected: PASS with zero failures.

- [ ] **Step 2: Run type checking**

Run `bun run typecheck` from `ui/`. Expected: exits 0 with no TypeScript errors.

- [ ] **Step 3: Inspect the diff**

Run `git diff --check HEAD^ HEAD` and `git status --short`. Expected: no whitespace errors and only the intended checkbox implementation files changed.
