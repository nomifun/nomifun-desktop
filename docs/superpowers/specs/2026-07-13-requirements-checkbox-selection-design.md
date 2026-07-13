# Requirements Checkbox Selection Design

## Goal

Replace the requirements workspace's plain native-looking selected checkbox state with a theme-aware dark fill, a centered white checkmark, and a compact transition animation.

## Scope

Only the requirement row checkbox and the "select all on this page" checkbox in the requirements filter toolbar are affected. The global Arco checkbox contract and checkboxes in other product areas remain unchanged.

## Visual behavior

- Idle: retain the existing theme-aware idle background and border.
- Hover: retain the existing interactive border treatment.
- Checked: fill the complete checkbox mask with `--control-selected-bg`; render the existing white checkmark within the mask, centered and visually inset from every edge.
- Indeterminate: preserve the existing full selected fill and centered white indicator.
- Focus: retain the global visible focus ring.
- Disabled: keep the global disabled selected color behavior.

The selected fill, border, and indicator animate over 160 ms with an ease-out curve. Respect `prefers-reduced-motion` by disabling the transition.

## Implementation

1. Add one `requirements-selection-checkbox` class to the two requirements-only Arco `Checkbox` instances.
2. Add scoped CSS next to the global theme control contract. It will target only the class's Arco mask/icon elements, keep overflow clipped, align the icon centrally, and animate selected-state properties.
3. Add an automated source-level contract test confirming both requirements controls use the scoped class and the stylesheet preserves all required visual-state selectors and reduced-motion handling.

## Verification

Run the focused Bun test for the new contract, the existing requirements filter tests, the existing theme control contract test, and `bun run typecheck` from `ui/`.
