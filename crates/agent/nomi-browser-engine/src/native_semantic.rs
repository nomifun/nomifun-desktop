//! Transport-neutral access to the existing bundled Playwright semantic core.
//! Scripts observe, locate and highlight. Input always belongs to the host driver.

pub fn initialization_expression() -> String {
    // The upstream constructor installs persistent input interceptors and a
    // document MutationObserver. A semantic-only instance never uses them:
    // real input belongs to our driver. Suppress those hooks so releasing a
    // remote object group also releases this instance and its observed nodes.
    format!(
        "{}\n((core) => {{ {}; return core; }})(new (class extends {}.InjectedScript {{ _setupGlobalListenersRemovalDetection() {{}} _setupHitTargetInterceptors() {{}} }})(globalThis, {}))",
        crate::injected::INJECTED_SOURCE,
        include_str!("native_stability.js"),
        crate::injected::INJECTED_GLOBAL,
        crate::injected::injected_options_json()
    )
}

pub const OBSERVE: &str = r#"function() {
    const result = this.incrementalAriaSnapshot(document.body, {mode:'ai',depth:12});
    const refs = this._lastAriaSnapshotForQuery?.elements;
    if (!refs || refs.size > 2000 || result.full.length > 200000) {
        this._lastAriaSnapshotForQuery = undefined;
        return {error:'limit'};
    }
    const elements = [];
    for (const [ref, el] of refs) {
        if (!el?.isConnected) continue;
        elements.push({ref_id:String(ref),role:this.utils.getAriaRole(el)||el.tagName.toLowerCase(),
            name:this.utils.getElementAccessibleName(el,false).slice(0,256),focused:el===document.activeElement});
    }
    return {content:result.full,elements};
}"#;

pub const LOCATE: &str = r#"async function(ref, editable) {
    const el = this._lastAriaSnapshotForQuery?.elements?.get(ref);
    if (!el?.isConnected) return {error:'stale'};
    const states = ['visible','stable','enabled'];
    if (editable) states.push('editable');
    const state = await this.__nomiCheckStates(el, states);
    if (state) return {error:'not_actionable'};
    const r = el.getBoundingClientRect();
    const x = Math.max(0,r.left) + (Math.min(innerWidth,r.right)-Math.max(0,r.left))/2;
    const y = Math.max(0,r.top) + (Math.min(innerHeight,r.bottom)-Math.max(0,r.top))/2;
    if (!(x>=0 && y>=0 && x<innerWidth && y<innerHeight) || r.width<=0 || r.height<=0)
        return {error:'not_actionable'};
    if (this.expectHitTarget({x,y},el) !== 'done') return {error:'not_actionable'};
    return {x,y};
}"#;

pub const HIGHLIGHT: &str = r#"function(x,y) {
    if (!Number.isFinite(x) || !Number.isFinite(y)) return false;
    if (!this.__nomiPointer?.isConnected) {
        const host = document.createElement('div');
        host.setAttribute('aria-hidden','true');
        host.setAttribute('data-nomi-agent-pointer','');
        host.style.cssText='all:initial!important;position:fixed!important;pointer-events:none!important;user-select:none!important;z-index:2147483647!important;width:28px!important;height:32px!important;margin:0!important;padding:0!important;border:0!important;transform:none!important;contain:layout style size!important;';
        const shadow=host.attachShadow({mode:'closed'});
        const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');
        svg.setAttribute('viewBox','0 0 28 32');
        svg.setAttribute('width','28'); svg.setAttribute('height','32');
        svg.style.cssText='display:block;pointer-events:none;overflow:visible';
        const path=document.createElementNS('http://www.w3.org/2000/svg','path');
        path.setAttribute('d','M1 1 L1 24 L7 18 L12 28 L17 25 L12 16 L22 16 Z');
        path.setAttribute('fill','#7065ee'); path.setAttribute('stroke','white');
        path.setAttribute('stroke-width','2'); path.setAttribute('stroke-linejoin','round');
        svg.appendChild(path); shadow.appendChild(svg);
        document.documentElement.appendChild(host);
        this.__nomiPointer=host;
    }
    this.__nomiPointer.style.setProperty('left',(x-1)+'px','important');
    this.__nomiPointer.style.setProperty('top',(y-1)+'px','important');
    return true;
}"#;

pub const CLEAR_HIGHLIGHT: &str = r#"function() {
    this.__nomiPointer?.remove(); this.__nomiPointer=undefined; return true;
}"#;

pub const IS_FOCUSED: &str = r#"function(ref) {
    const el=this._lastAriaSnapshotForQuery?.elements?.get(ref);
    let active=document.activeElement;
    while(active?.shadowRoot?.activeElement) active=active.shadowRoot.activeElement;
    return !!el?.isConnected && el===active;
}"#;

// These helpers only inspect the select and guard node identity. Selection is
// performed by the native keyboard driver, never by assigning selected/value.
pub const PREPARE_SELECT: &str = r#"function(ref, labels) {
    this.__nomiSelect?.cleanup?.();
    this.__nomiSelect = undefined;
    const el = this._lastAriaSnapshotForQuery?.elements?.get(ref);
    if (!el?.isConnected) return {error:'stale'};
    if (el.tagName !== 'SELECT' || getComputedStyle(el).appearance==='base-select') return {error:'unsupported'};
    if (el.matches(':disabled')) return {error:'not_actionable'};
    const options = Array.from(el.options);
    if (options.length > 512 || labels.length > 512) return {error:'unsupported'};
    const name = option => this.utils.getElementAccessibleName(option,false).replace(/\s+/g,' ').trim();
    const disabled = option => option.disabled || (option.parentElement?.tagName === 'OPTGROUP' && option.parentElement.disabled);
    const hidden = option => {
        for (let node=option; node && node!==el; node=node.parentElement)
            if (getComputedStyle(node).display==='none') return true;
        return false;
    };
    const names = labels.map(label => label.replace(/\s+/g,' ').trim());
    if ((!el.multiple && names.length !== 1) || new Set(names).size !== names.length)
        return {error:'not_actionable'};
    const desired_indices = [];
    for (const label of names) {
        const matches = options.map((option,index) => name(option)===label ? index : -1).filter(index => index>=0);
        if (matches.length !== 1) return {error:'not_actionable'};
        const index = matches[0];
        if ((disabled(options[index]) || hidden(options[index])) && !(el.multiple && options[index].selected)) return {error:'not_actionable'};
        desired_indices.push(index);
    }
    desired_indices.sort((a,b)=>a-b);
    const enabled_indices = options.map((option,index) => disabled(option) || hidden(option) ? -1 : index).filter(index => index>=0);
    const selected_indices = options.map((option,index) => option.selected ? index : -1).filter(index => index>=0);
    const fixed_selected = selected_indices.filter(index=>disabled(options[index]) || hidden(options[index]));
    const reset_selection = el.multiple && fixed_selected.some(index=>!desired_indices.includes(index));
    // Plain Home clears prior selection before entering non-contiguous mode.
    // An inaccessible selected row cannot be cleared while retaining another
    // inaccessible row, because native input cannot select that row again.
    if (reset_selection && (!enabled_indices.length || fixed_selected.some(index=>desired_indices.includes(index))))
        return {error:'not_actionable'};
    const writingMode=getComputedStyle(el).writingMode;
    const listbox=el.multiple || el.size>1;
    const next_key=!listbox || writingMode==='horizontal-tb' ? 'ArrowDown' :
        (writingMode==='vertical-rl' || writingMode==='sideways-rl' ? 'ArrowLeft' : 'ArrowRight');
    const previous_key=next_key==='ArrowDown' ? 'ArrowUp' : (next_key==='ArrowLeft' ? 'ArrowRight' : 'ArrowLeft');
    const signature = option => JSON.stringify([name(option),option.value,!!disabled(option),hidden(option)]);
    this.__nomiSelect = {el,options,multiple:el.multiple,size:el.size,writingMode,signature,signatures:options.map(signature)};
    return {multiple:el.multiple,enabled_indices,selected_indices,desired_indices,next_key,previous_key,reset_selection};
}"#;

pub const SELECT_NODE: &str = r#"function() { return this.__nomiSelect?.el; }"#;

pub const SELECT_STATE: &str = r#"function() {
    const plan=this.__nomiSelect;
    if (!plan?.el?.isConnected || plan.el.multiple!==plan.multiple || plan.el.size!==plan.size ||
        getComputedStyle(plan.el).writingMode!==plan.writingMode || plan.el.matches(':disabled')) return {error:'stale'};
    const options=Array.from(plan.el.options);
    if (options.length!==plan.options.length || options.some((option,index) => {
        return option!==plan.options[index] || plan.signature(option)!==plan.signatures[index];
    })) return {error:'stale'};
    return {focused:document.activeElement===plan.el,
        selected_indices:options.map((option,index)=>option.selected ? index : -1).filter(index=>index>=0)};
}"#;

// Observe completion of the actual trusted key event. A page that cancels an
// arrow key must not make the driver toggle an option at an assumed new position.
pub const ARM_SELECT_KEY: &str = r#"function() {
    const plan=this.__nomiSelect;
    if (!plan?.el?.isConnected) return false;
    plan.cleanup?.();
    plan.keyEvent=undefined;
    const listener=event=>{ plan.keyEvent=event; };
    plan.cleanup=()=>plan.el.removeEventListener('keydown',listener,true);
    plan.el.addEventListener('keydown',listener,{capture:true,once:true,passive:true});
    return true;
}"#;

pub const SELECT_KEY_ACCEPTED: &str = r#"function() {
    const plan=this.__nomiSelect, event=plan?.keyEvent;
    return !!event?.isTrusted && event.target===plan.el && !event.defaultPrevented;
}"#;

pub const CLEAR_SELECT: &str = r#"function() {
    this.__nomiSelect?.cleanup?.();
    this.__nomiSelect=undefined;
    return true;
}"#;
