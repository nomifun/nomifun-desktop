// The subtitle window has only keyboard capabilities and does not bootstrap the
// authenticated workbench, Agent runtime, or its application command adapters.
if (window.location.hash.split('?')[0] === '#/keyboard-subtitle') {
  void import('./pages/keyboardSubtitle/main');
} else {
  void import('./main');
}
