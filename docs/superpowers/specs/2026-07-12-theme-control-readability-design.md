# Theme Control Readability Design

## Goal

Make every built-in theme readable and visually coherent in light and dark modes, with reliable state contrast for interactive controls.

## Root cause

Preset CSS is injected after the application styles and every declaration is promoted to `!important`. Arco’s checkbox, radio, and switch selected states use `--primary-6` for their fill but hard-code a light indicator. The classic dark preset deliberately uses a near-white primary, so its selected controls can render a light indicator on a light fill. A single primary scale is also responsible for buttons, which prevents safely darkening it just for controls.

## Design

1. Add a component-specific semantic token set to the preset-theme contract: selected surface/foreground, idle surface/border, hover surface, disabled selected surface, and focus ring.
2. Supply those tokens in both modes of all five built-in themes. They may follow a theme’s palette, but the selected foreground and surface must remain visibly distinct.
3. Add one global control-contract stylesheet after the preset stylesheet. It maps the semantic tokens to Arco Checkbox, Radio, Switch, checkable Tag, and Tabs states. It covers default, hover, selected, focus-visible, disabled, loading, and indeterminate states without changing component geometry.
4. Retain each theme’s existing primary scale for buttons, links, and branding. Controls use the new semantic accent independently.
5. Audit page-specific controls in assistants/skills, companion migration, scheduled-task creation, requirements, and knowledge pages. Add only scoped fixes where a custom component bypasses standard Arco markup.

## Verification

- A theme-contract test requires the new semantic tokens in every built-in preset, in both light and dark blocks.
- A stylesheet test requires all core control state selectors and asserts no selected state uses the generic primary scale.
- The component showcase renders Checkbox, Radio, Switch, checkable Tag, and Tabs in their relevant states for visual regression review.
- Run the theme-contract script, targeted Bun tests, UI typecheck, and the UI production build.

## Constraints

- Preserve layout and existing interaction behavior.
- Do not alter the default light/dark base palette outside the selected controls.
- Do not create a git commit for this work.
