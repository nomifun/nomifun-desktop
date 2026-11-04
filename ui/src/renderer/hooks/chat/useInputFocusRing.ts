export const useInputFocusRing = () => {
  return {
    // Keep the focus treatment in CSS so custom presets and live theme changes
    // update both the Composer and plan editor without a React theme branch.
    activeBorderColor: 'var(--control-selected-bg, var(--color-primary))',
    inactiveBorderColor: 'var(--control-idle-border, var(--color-border-3))',
    activeShadow: '0px 2px 20px var(--control-focus-ring, color-mix(in srgb, var(--color-primary) 26%, transparent))',
  };
};
