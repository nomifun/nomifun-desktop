# Theme System

The renderer theme system separates **light/dark mode** from **color scheme**.
This keeps mode switching simple while leaving room for future palettes.

## Dimensions

| Dimension | Controlled by | Values | DOM attribute |
| --- | --- | --- | --- |
| Theme mode | `useTheme` | `light`, `dark` | `<html data-theme>` and `<body arco-theme>` |
| Color scheme | `useColorScheme` | `default` | `<html data-color-scheme>` |

`useColorScheme` lives at `ui/src/renderer/hooks/ui/useColorScheme.ts`.

## File Structure

```text
ui/src/renderer/styles/themes/
├── index.css
├── base.css
└── default-color-scheme.css
```

`index.css` imports the base styles and the available color-scheme files.

## Token Naming

### Legacy-Compatible Brand Scale

`--aou-1` through `--aou-10` remain as compatibility tokens for older styles and
components. They should be treated as the current default brand color scale, not
as active AOU product branding.

### Core Tokens

- `--bg-base`, `--bg-1`, `--bg-2`, `--bg-3`
- `--bg-hover`, `--bg-active`
- `--text-primary`, `--text-secondary`, `--text-disabled`
- `--primary`, `--success`, `--warning`, `--danger`
- `--brand`, `--brand-light`, `--brand-hover`

Component-specific tokens such as `--message-user-bg` and
`--workspace-btn-bg` are allowed when a generic semantic token is not precise
enough.

## Adding A Color Scheme

1. Add a new `*-color-scheme.css` file beside `default-color-scheme.css`.
2. Define both light and dark variants.
3. Import the file from `index.css`.
4. Update the `ColorScheme` type and option list in
   `ui/src/renderer/hooks/ui/useColorScheme.ts`.
5. Add selector UI and translations if the scheme is user-facing.
6. Run the theme contract check:

   ```bash
   bun run check:theme
   ```

## Rules

- Always define light and dark values together.
- Keep background tokens neutral enough for long-running work surfaces.
- Prefer semantic names for new component tokens.
- Do not introduce product-brand names that are not intended to remain public.
