# Renderer Styling

[简体中文](styling.zh.md)

Use semantic tokens and existing Arco/UnoCSS patterns. The shared renderer
supports desktop windows and desktop WebUI browsers at 880x600 or larger.
Run `bun run check:desktop-ui-boundary` after renderer or UI-rule changes.

## UnoCSS utilities

The executable definitions are in [ui/uno.config.ts](../../ui/uno.config.ts).
The [dead utility checker](../../scripts/check-dead-css-utilities.mjs) rejects
known invalid forms and compiles utility-shaped tokens through the real generator
to detect classes that emit no CSS.

| Intent | Use | Avoid |
| --- | --- | --- |
| Background | `bg-base`, `bg-1` through `bg-6`, `bg-8` through `bg-10` | `bg-bg-N`, `bg-0`, `bg-7` |
| Text | `text-t-primary`, `text-t-secondary` | `text-text-primary` |
| Semantic ramp | `text-danger-6`, `bg-primary-6` | `text-[rgb(var(--danger-6))]` |
| Explicit alpha | `bg-[rgba(var(--primary-6),0.12)]` | `bg-primary-6/12` |
| Border color | `border-arco-2`, `border-[var(--border-base)]` | `border-border-2`, `border-b-base`, `border-b-light` |
| Border width | `border-2px`, `border-b-2px` | Treating `border-2` or `border-b-2` as a width |

Arco color ramps contain comma-separated RGB triplets. UnoCSS's injected
slash-alpha makes `rgb(var(--danger-6) / ...)` invalid after expansion.
Use the semantic rule or an explicit `rgba(...)` with alpha. Nonsemantic
palettes and ring/outline/directional color utilities need explicit alpha.

The `-b-` direction trap consumes the direction before resolving the color:
`border-b-base` selects `--bg-base`, not `--border-base`.
Numeric theme keys are colors; append `px` for unambiguous widths.

There is no global border reset. Set width, style and color explicitly:

```tsx
<div className='border border-solid border-arco-2' />
<div className='border-b border-b-solid border-b-[var(--border-base)]' />
```

Directional widths need directional styles. Using `border-solid` with only a
bottom width lets the other sides fall back to `border-width: medium`.
Do not add a global `* { border-width: 0; border-style: solid; }` reset:
it changes native control borders and existing CSS throughout the renderer.

## Theme presets

The built-in presets are Classic (the default) and Frosted Glass. Custom CSS
themes remain available alongside them.

Theme mode and color scheme are separate concerns. Prefer semantic tokens in
[styles/themes](../../ui/src/renderer/styles/themes); `--aou-1..10` is the
current brand scale's compatibility name.

[AppThemeRuntime](../../ui/src/renderer/components/layout/AppThemeRuntime.tsx)
injects custom CSS through
[applyCustomCss](../../ui/src/renderer/utils/theme/applyCustomCss.ts), which
adds `!important` to declarations outside keyframes. Presets must:

- Define light values on `:root, body`, followed by matching dark values on
  `[data-theme='dark'], [data-theme='dark'] body`. Arco defines tokens on
  `body`; overriding only `:root` is insufficient.
- Cover the full App/Arco token sets with identical light/dark keys.
  The exact required sets live in
  [check-theme-contract.mjs](../../scripts/check-theme-contract.mjs).
- Keep `--primary-rgb` and `--primary-1..7` as RGB triplets.
  Supply a literal accent color for the sidebar theme selector.
- Restrict decorative styles to specific targets and appearance. Do not add
  layout properties, broad selectors, external resources or variables inside
  `@media`.
- Keep text contrast at least 4.5:1. Content popovers/dropdowns/select menus
  need sufficiently solid backgrounds (alpha at least 0.86). Feedback and
  tooltip backgrounds must be opaque.
- Keep the message layout wrapper `.message-item` free of preset backgrounds,
  glass borders and blur. Use semantic bubble tokens or specific components.

## Validation

Run `bun run check:dead-css` and `bun run check:theme` for styling changes.
Inspect affected controls in both light and dark modes. For changed behavior,
run the focused component/contract tests; a documentation-only edit needs
`bun run check:docs` and `git diff --check`.
