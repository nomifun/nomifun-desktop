// Native WebViews can stop painting while hidden without stopping page timers.
// Observe actual geometry on a bounded clock; never force a frame or synthesize
// input. Keep the upstream visible/enabled/editable checks and exact node guards.
core.__nomiCheckStates = async function(node, states) {
  const remaining = states.filter(state => state !== 'stable');
  const initial = await this.checkElementStates(node, remaining);
  if (initial || !states.includes('stable')) return initial;
  const builtins = this.utils.builtins;
  const element = this.retarget(node, 'no-follow-label');
  if (!element?.isConnected) return 'error:notconnected';
  const stable = await new Promise(resolve => {
    let timer, frame, deadline, finished = false, lastTime, lastRect, matches = 0;
    const finish = result => {
      if (finished) return;
      finished = true;
      builtins.clearTimeout(timer);
      builtins.clearTimeout(deadline);
      builtins.cancelAnimationFrame(frame);
      resolve(result);
    };
    const schedule = () => {
      timer = builtins.setTimeout(sample, 20);
      frame = builtins.requestAnimationFrame(sample);
    };
    const sample = () => {
      if (finished) return;
      builtins.clearTimeout(timer);
      builtins.cancelAnimationFrame(frame);
      try {
        if (!node.isConnected || !element.isConnected || this.retarget(node, 'no-follow-label') !== element) {
          finish('error:notconnected'); return;
        }
        const now = builtins.performance.now();
        if (lastTime !== undefined && now - lastTime < 15) { schedule(); return; }
        const rect = element.getBoundingClientRect();
        const current = [rect.x, rect.y, rect.width, rect.height];
        if (!current.every(Number.isFinite)) { finish({missingState:'stable'}); return; }
        if (lastRect && current.some((value, index) => value !== lastRect[index])) {
          finish({missingState:'stable'}); return;
        }
        if (lastRect && ++matches >= 2) { finish(undefined); return; }
        lastRect = current; lastTime = now;
        schedule();
      } catch { finish({missingState:'stable'}); }
    };
    deadline = builtins.setTimeout(() => finish({missingState:'stable'}), 2000);
    sample();
  });
  if (stable) return stable;
  if (!node.isConnected || !element.isConnected || this.retarget(node, 'no-follow-label') !== element) return 'error:notconnected';
  return this.checkElementStates(node, remaining);
};
