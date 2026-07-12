# Theme Control Readability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every built-in visual theme render readable, coherent interactive controls in both light and dark modes.

**Architecture:** Built-in themes provide a dedicated semantic control palette. A late-injected global stylesheet consumes that palette for Arco control states so it remains authoritative after preset CSS is injected with `!important`.

**Tech Stack:** React 19, TypeScript, Arco Design, CSS custom properties, Bun tests, Vite.

## Global Constraints

- Preserve layout and component behavior.
- Keep theme primary-button styling independent from control selected-state colors.
- Supply every new token in every built-in preset’s light and dark blocks.
- Do not create a git commit.

---

### Task 1: Lock the contract with failing tests

**Files:**
- Create: `ui/src/renderer/styles/themeControlContract.test.ts`
- Modify: `scripts/check-theme-contract.mjs`

- [ ] **Step 1: Write failing tests**

Assert each `PRESET_THEMES` stylesheet contains each `--control-*` token exactly twice and assert the control stylesheet contains the selected Checkbox, Radio, Switch, Tag, Tabs, focus, and disabled selectors.

- [ ] **Step 2: Run the targeted test and confirm failure**

Run: `bun test ui/src/renderer/styles/themeControlContract.test.ts`
Expected: failure because the token set and control stylesheet do not exist.

### Task 2: Add semantic themes and control layer

**Files:**
- Create: `ui/src/renderer/styles/theme-control-contract.css`
- Modify: `ui/src/renderer/components/layout/Layout.tsx`
- Modify: `ui/src/renderer/pages/companion/index.tsx`
- Modify: `ui/src/renderer/pages/memoryPanel/index.tsx`
- Modify: `ui/src/renderer/pages/settings/DisplaySettings/presets/*.css`
- Modify: `scripts/check-theme-contract.mjs`

- [ ] **Step 1: Add the new token set**

Add semantic control tokens to both blocks of all presets and require them from the checker.

- [ ] **Step 2: Add the contract stylesheet after each injected preset**

Keep the preset style id unchanged and append a second, deterministic style element after it in the main and companion windows.

- [ ] **Step 3: Apply core control states**

Map the semantic token set to Arco Checkbox, Radio, Switch, checkable Tag, and Tabs default/hover/selected/focus/disabled states.

- [ ] **Step 4: Run targeted tests and theme contract**

Run: `bun test ui/src/renderer/styles/themeControlContract.test.ts && bun scripts/check-theme-contract.mjs`
Expected: pass.

### Task 3: Expand visual coverage and verify application behavior

**Files:**
- Modify: `ui/src/renderer/pages/TestShowcase.tsx`

- [ ] **Step 1: Add representative core controls**

Render Checkbox, Radio, Switch, checkable Tag, and Tabs in the showcase with selected and disabled variants.

- [ ] **Step 2: Run complete validation**

Run: `bun test ui/src/renderer/styles/themeControlContract.test.ts ui/src/renderer/styles/buttonThemeContract.test.ts && bun scripts/check-theme-contract.mjs && bun run --filter=./ui typecheck && bun run --filter=./ui build`
Expected: all commands exit 0.
