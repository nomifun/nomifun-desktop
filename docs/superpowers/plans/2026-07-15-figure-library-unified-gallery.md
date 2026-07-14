# Figure Library Unified Gallery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render every figure-library asset and the create entry as a consistently sized, polished gallery card.

**Architecture:** Keep the existing `FigureTile` component and figure CRUD behavior intact. Replace proportional grid/card sizing with shared card, preview, and footer constraints directly in `FigureLibraryPage.tsx`; the trailing creation control uses the same constraints. Extend the source-level visual regression test to guard the three shared dimensions.

**Tech Stack:** React 19, TypeScript, UnoCSS utility classes, Bun test runner.

## Global Constraints

- Do not change figure persistence, CRUD callbacks, image URLs, edit modal, or in-use deletion protection.
- Each saved card and the create entry use a fixed `184px × 234px` outer size.
- Each saved card uses a `190px` preview region and a `44px` footer/name region without a dividing border.
- The create entry is a visual exception inside that fixed outer geometry: a plain light surface with a centered plus icon and one label beneath it, without a dashed border or footer divider.
- Do not create a git commit; the user explicitly requested uncommitted changes.

---

### Task 1: Lock the shared card geometry with a failing regression test

**Files:**
- Modify: `ui/src/renderer/pages/nomi/figureActionsVisual.test.ts`

**Interfaces:**
- Consumes: `FigureLibraryPage.tsx` source text.
- Produces: a source-level regression test requiring both saved and create cards to reference `figure-library-card`, `figure-library-card-preview`, and `figure-library-card-footer`.

- [ ] **Step 1: Write the failing test**

Add this test to `ui/src/renderer/pages/nomi/figureActionsVisual.test.ts`:

```ts
test('uses one fixed gallery-card geometry for figures and the creation entry', () => {
  const library = readSource(new URL('./FigureLibraryPage.tsx', import.meta.url));

  expect(library.includes('figure-library-card w-184px h-244px')).toBe(true);
  expect(library.includes('figure-library-card-preview h-190px')).toBe(true);
  expect(library.includes('figure-library-card-footer h-54px')).toBe(true);
  expect(library.includes('figure-library-create-card')).toBe(true);
  expect(library.includes('gridTemplateColumns: \'repeat(auto-fill, 184px)\'')).toBe(true);
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ui && bun test src/renderer/pages/nomi/figureActionsVisual.test.ts`

Expected: FAIL because the gallery-card class names and fixed 184px grid column are not yet present.

- [ ] **Step 3: Keep the failing test focused**

Do not add assertions about hover shadows, image URLs, or CRUD behavior; those are already covered by the component and existing test cases.

### Task 2: Give saved and creation cards a shared fixed structure

**Files:**
- Modify: `ui/src/renderer/pages/nomi/FigureLibraryPage.tsx:35-60`

**Interfaces:**
- Consumes: existing `fig`, `baseUrl`, `onUpdate`, and `onDelete` props unchanged.
- Produces: a saved card with `figure-library-card`, `figure-library-card-preview`, and `figure-library-card-footer` class markers for the regression test.

- [ ] **Step 1: Change the saved card outer frame**

Replace the opening saved-card `div` class with:

```tsx
className='figure-library-card group relative flex flex-col w-184px h-244px shrink-0 overflow-hidden rd-16px bg-fill-2 border border-solid border-[var(--color-border-2)] shadow-[0_1px_2px_rgba(15,23,42,0.04)] transition-all duration-200 hover:-translate-y-2px hover:shadow-[0_10px_28px_rgba(var(--primary-rgb),0.16)] hover:border-[var(--color-primary)]'
```

- [ ] **Step 2: Fix the saved-card preview and footer regions**

Use these classes for the image wrapper and the name wrapper, preserving their existing children:

```tsx
className='figure-library-card-preview flex h-190px shrink-0 items-center justify-center overflow-hidden'
```

```tsx
className='figure-library-card-footer flex h-54px shrink-0 items-center gap-6px px-12px border-t border-solid border-[var(--color-border-2)] bg-fill-2'
```

Keep `style={CHECKER_BG}` on the preview and keep the image `object-contain`, max dimensions, and accessibility text unchanged.

- [ ] **Step 3: Align the grid to the fixed card width**

Replace the gallery grid style with:

```tsx
style={{ gridTemplateColumns: 'repeat(auto-fill, 184px)' }}
```

Use `justify-start` on the grid container so card widths remain stable and unused horizontal area stays at the end of each row.

- [ ] **Step 4: Replace the create entry with the same frame**

Use a `button` class containing `figure-library-card figure-library-create-card`, `w-184px`, `h-244px`, a plain `bg-fill-1` surface, and the existing hover/focus behavior. Center a `w-44px h-44px rd-full bg-fill-3` plus icon and the `t('nomi.customFigure.createNew')` label in one vertical stack. Do not use `border-dashed`, `figure-library-card-preview`, or `figure-library-card-footer` inside the create entry.

- [ ] **Step 5: Run the focused test to verify it passes**

Run: `cd ui && bun test src/renderer/pages/nomi/figureActionsVisual.test.ts`

Expected: PASS with the new geometry test and existing visual-action tests all green.

### Task 3: Verify type safety and the uncommitted diff

**Files:**
- Verify: `ui/src/renderer/pages/nomi/FigureLibraryPage.tsx`
- Verify: `ui/src/renderer/pages/nomi/figureActionsVisual.test.ts`

**Interfaces:**
- Consumes: final JSX class structure from Task 2.
- Produces: verified workspace changes only; no git commit.

- [ ] **Step 1: Run the TypeScript check**

Run: `cd ui && bun run typecheck`

Expected: exit code 0 with no TypeScript diagnostics.

- [ ] **Step 2: Inspect the final diff**

Run: `git diff --check && git diff -- ui/src/renderer/pages/nomi/FigureLibraryPage.tsx ui/src/renderer/pages/nomi/figureActionsVisual.test.ts docs/superpowers/specs/2026-07-15-figure-library-unified-gallery-design.md docs/superpowers/plans/2026-07-15-figure-library-unified-gallery.md`

Expected: no whitespace errors; only the intended UI, test, specification, and plan changes appear.

- [ ] **Step 3: Do not commit**

Leave the verified changes unstaged and uncommitted, per the user request.
