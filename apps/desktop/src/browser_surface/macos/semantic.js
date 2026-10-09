// Executed only in the host's isolated WKContentWorld. DOM writes are semantic
// operations; dispatched input/change events are untrusted and grant no user activation.
(request) => {
    "use strict";
    const slot = "__nomifunWKSemanticV1";
    const fail = error => ({ error });
    const clean = (text, limit = 512) => String(text ?? "").slice(0, limit * 4).replace(/\s+/g, " ").trim().slice(0, limit);
    const parent = element => element.parentElement || element.getRootNode()?.host || null;
    const rect = element => {
        if (!element?.isConnected || element.ownerDocument !== document) return null;
        let box = element.getBoundingClientRect();
        let left = Math.max(0, box.left), top = Math.max(0, box.top);
        let right = Math.min(innerWidth, box.right), bottom = Math.min(innerHeight, box.bottom);
        let depth = 0;
        for (let node = element; node; node = parent(node)) {
            if (++depth > 128) return null;
            const style = getComputedStyle(node);
            if (style.display === "none" || style.visibility === "hidden" || style.visibility === "collapse" || Number(style.opacity) === 0) return null;
            if (node !== element && node !== document.body && node !== document.documentElement) {
                box = node.getBoundingClientRect();
                if (/(auto|scroll|hidden|clip)/.test(style.overflowX)) { left = Math.max(left, box.left); right = Math.min(right, box.right); }
                if (/(auto|scroll|hidden|clip)/.test(style.overflowY)) { top = Math.max(top, box.top); bottom = Math.min(bottom, box.bottom); }
            }
        }
        return right > left && bottom > top ? {left, top, right, bottom} : null;
    };
    const enabled = element => {
        if (element.matches(":disabled")) return false;
        let depth = 0;
        for (let node = element; node; node = parent(node)) {
            if (++depth > 128) return false;
            if (node.inert || node.getAttribute("aria-disabled") === "true") return false;
        }
        return true;
    };
    const contains = (element, hit) => {
        let depth = 0;
        for (let node = hit; node && ++depth <= 128; node = parent(node)) if (node === element) return true;
        return false;
    };
    const actionable = element => {
        const box = rect(element);
        if (!box || !enabled(element)) return false;
        const x = (box.left + box.right) / 2, y = (box.top + box.bottom) / 2;
        let hit = document.elementFromPoint(x, y);
        while (hit?.shadowRoot) {
            const child = hit.shadowRoot.elementFromPoint?.(x, y);
            if (!child || child === hit) break;
            hit = child;
        }
        return contains(element, hit);
    };
    const text = (element, limit = 512) => {
        const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
        let out = "", node, count = 0;
        while ((node = walker.nextNode()) && count++ < 512 && out.length < limit * 2) {
            if (!node.parentElement?.closest("script,style,noscript,template")) out += ` ${node.textContent.slice(0, limit)}`;
        }
        return clean(out, limit);
    };
    const name = element => {
        const explicit = element.getAttribute("aria-label");
        if (explicit) return clean(explicit);
        const labelled = element.getAttribute("aria-labelledby");
        if (labelled) {
            const root = element.getRootNode();
            const value = labelled.split(/\s+/).slice(0, 16).map(id => {
                const label = root.getElementById?.(id); return label ? text(label) : "";
            }).join(" ");
            if (value.trim()) return clean(value);
        }
        if (element.labels?.length) return clean(Array.from(element.labels).slice(0, 8).map(label => text(label)).join(" "));
        if (element instanceof HTMLInputElement) {
            if (["button", "submit", "reset"].includes(element.type)) return clean(element.value || element.type);
            return clean(element.placeholder || element.title || element.name);
        }
        if (element instanceof HTMLTextAreaElement) return clean(element.placeholder || element.title || element.name);
        if (element instanceof HTMLSelectElement) return clean(element.title || element.name);
        if (element instanceof HTMLImageElement) return clean(element.alt || element.title);
        return text(element) || clean(element.title);
    };
    const role = element => {
        const explicit = element.getAttribute("role");
        if (explicit) return clean(explicit.split(/\s+/)[0], 64);
        const tag = element.tagName.toLowerCase();
        if (tag === "a" && element.hasAttribute("href")) return "link";
        if (tag === "button") return "button";
        if (tag === "textarea" || element.isContentEditable) return "textbox";
        if (tag === "select") return element.multiple ? "listbox" : "combobox";
        if (tag === "input") return ({checkbox:"checkbox", radio:"radio", button:"button", submit:"button", reset:"button", range:"slider", file:"file_input", hidden:null})[element.type] ?? (element.type === "hidden" ? null : "textbox");
        if (/^h[1-6]$/.test(tag)) return "heading";
        if (tag === "img" && element.alt) return "img";
        if (element.tabIndex >= 0) return "generic";
        const style = getComputedStyle(element);
        if ((element.scrollHeight > element.clientHeight && /(auto|scroll)/.test(style.overflowY)) || (element.scrollWidth > element.clientWidth && /(auto|scroll)/.test(style.overflowX))) return "region";
        return null;
    };
    const focused = element => {
        let active = document.activeElement;
        while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
        return active === element;
    };
    const identity = (element, documentRoot) => {
        // Keep the actual node as the target, but reject nodes repurposed in
        // place after observation. Values, selection and focus remain dynamic.
        // The document scroll target must not fingerprint the whole page text.
        const control = element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement
            || element instanceof HTMLSelectElement || element instanceof HTMLButtonElement;
        const typed = element instanceof HTMLInputElement || element instanceof HTMLButtonElement;
        const link = element instanceof HTMLAnchorElement || element instanceof HTMLAreaElement;
        return JSON.stringify([
            documentRoot ? "document" : role(element),
            documentRoot ? clean(document.title) : name(element),
            typed ? element.type : null,
            control ? element.name : null,
            link ? element.href : null,
            enabled(element),
            element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement ? element.readOnly : false,
        ]);
    };

    if (request.operation === "observe") {
        const state = { nonce: request.nonce, document, elements: new Map() };
        // Re-observe invalidates all earlier references even if this pass fails.
        globalThis[slot] = state;
        const elements = [], lines = [];
        let length = 0, visited = 0, truncated = false;
        const unobservedFrames = new Set(document.querySelectorAll("iframe,frame"));
        const append = line => {
            const available = request.max_content - length;
            if (available <= 1) { truncated = true; return; }
            const value = line.slice(0, available - 1); lines.push(value); length += value.length + 1;
            if (value.length < line.length) truncated = true;
        };
        const add = (element, elementRole, elementName, documentRoot = false) => {
            if (elements.length >= request.max_elements) { truncated = true; return; }
            const ref_id = `wk.${state.nonce}.${elements.length}`;
            state.elements.set(ref_id, {element, documentRoot, identity:identity(element, documentRoot)});
            const label = clean(elementName, request.max_name);
            elements.push({ref_id, role:elementRole, name:label, focused:focused(element)});
            append(`[${ref_id}] ${elementRole} ${JSON.stringify(label)}${enabled(element) ? "" : " [disabled]"}`);
            if (element instanceof HTMLSelectElement) {
                for (const option of Array.from(element.options).slice(0, 100)) {
                    append(`  option ${JSON.stringify(clean(option.label))}${option.selected ? " [selected]" : ""}${option.disabled || option.parentElement?.disabled ? " [disabled]" : ""}`);
                }
            }
        };
        add(document.scrollingElement || document.documentElement, "document", document.title, true);
        const visit = root => {
            const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
                acceptNode(node) {
                    if (node.nodeType === Node.ELEMENT_NODE && /^(SCRIPT|STYLE|NOSCRIPT|TEMPLATE|HEAD)$/.test(node.tagName)) return NodeFilter.FILTER_REJECT;
                    return NodeFilter.FILTER_ACCEPT;
                }
            });
            let node;
            while ((node = walker.nextNode())) {
                if (++visited > 12_000) { truncated = true; return; }
                if (node.nodeType === Node.TEXT_NODE) {
                    const value = clean(node.textContent, 2048);
                    if (value && node.parentElement && rect(node.parentElement)) append(value);
                    continue;
                }
                // No cross-frame result is invented: all child documents are
                // explicitly reported as outside this first-version observer.
                if (node.tagName === "IFRAME" || node.tagName === "FRAME") unobservedFrames.add(node);
                if (rect(node)) {
                    const elementRole = role(node);
                    if (elementRole) add(node, elementRole, name(node));
                }
                if (node.shadowRoot) visit(node.shadowRoot);
            }
        };
        visit(document.body || document.documentElement);
        // Frames past the traversal cap must still contribute to the coverage.
        const unobserved_frames = unobservedFrames.size;
        if (truncated) {
            const marker = "\n[Observation truncated at its size limit]";
            const content = lines.join("\n").slice(0, Math.max(0, request.max_content - marker.length)) + marker;
            return {content, elements, unobserved_frames};
        }
        return {content:lines.join("\n"), elements, unobserved_frames};
    }

    const state = globalThis[slot];
    if (!state || state.nonce !== request.nonce || state.document !== document) return fail("stale");
    const observed = state.elements.get(request.ref_id);
    const element = observed?.element;
    if (!element?.isConnected || element.ownerDocument !== document) return fail("stale");
    if (identity(element, observed.documentRoot) !== observed.identity) return fail("stale");
    if (!actionable(element)) return fail("not_actionable");
    let sent = false;
    try {
        if (request.operation === "click") {
            // File choosers and user-activation-only features remain manual.
            if (!(element instanceof HTMLElement) || (element instanceof HTMLInputElement && element.type === "file")) return fail("unsupported");
            const labelled = element.closest("label")?.control;
            if (labelled instanceof HTMLInputElement && labelled.type === "file") return fail("unsupported");
            sent = true;
            HTMLElement.prototype.click.call(element);
        } else if (request.operation === "type") {
            if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement)) return fail("unsupported");
            if (element.readOnly) return fail("not_actionable");
            if (element instanceof HTMLInputElement && !["text", "search", "tel", "url", "email", "password"].includes(element.type)) return fail("unsupported");
            if (element.maxLength >= 0 && request.text.length > element.maxLength) return fail("not_actionable");
            const prototype = element instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
            const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
            if (!setter) return fail("unsupported");
            sent = true;
            element.focus({preventScroll:true});
            if (!element.isConnected || !actionable(element) || !focused(element)
                || identity(element, observed.documentRoot) !== observed.identity) return fail("interrupted");
            setter.call(element, request.text);
            // Framework listeners see the actual value change. These events are
            // intentionally untrusted; no isTrusted/user-activation claim exists.
            element.dispatchEvent(new Event("input", {bubbles:true, composed:true}));
            element.dispatchEvent(new Event("change", {bubbles:true}));
            if (!element.isConnected || element.value !== request.text) return fail("interrupted");
        } else if (request.operation === "select") {
            if (!(element instanceof HTMLSelectElement)) return fail("unsupported");
            if (element.options.length > 4096) return fail("limit");
            if (!element.multiple && request.labels.length !== 1) return fail("not_actionable");
            const options = Array.from(element.options), desired = [];
            for (const label of request.labels) {
                const matches = options.filter(option => clean(option.label) === label);
                if (matches.length !== 1 || matches[0].disabled || matches[0].parentElement?.disabled) return fail("not_actionable");
                desired.push(matches[0]);
            }
            const setter = Object.getOwnPropertyDescriptor(HTMLOptionElement.prototype, "selected")?.set;
            if (!setter) return fail("unsupported");
            sent = true;
            element.focus({preventScroll:true});
            if (!element.isConnected || !actionable(element)
                || identity(element, observed.documentRoot) !== observed.identity
                || options.length !== element.options.length
                || options.some((option, index) => element.options[index] !== option)
                || desired.some((option, index) => clean(option.label) !== request.labels[index]
                    || option.disabled || option.parentElement?.disabled)) return fail("interrupted");
            for (const option of options) setter.call(option, desired.includes(option));
            element.dispatchEvent(new Event("input", {bubbles:true, composed:true}));
            element.dispatchEvent(new Event("change", {bubbles:true}));
            if (!element.isConnected || options.some(option => option.selected !== desired.includes(option))) return fail("interrupted");
        } else if (request.operation === "scroll") {
            let scroll = element;
            while (scroll && scroll !== document.scrollingElement) {
                const style = getComputedStyle(scroll);
                const vertical = request.delta_y && scroll.scrollHeight > scroll.clientHeight && /(auto|scroll)/.test(style.overflowY);
                const horizontal = request.delta_x && scroll.scrollWidth > scroll.clientWidth && /(auto|scroll)/.test(style.overflowX);
                if (vertical || horizontal) break;
                scroll = parent(scroll);
            }
            scroll ||= document.scrollingElement;
            if (!scroll) return fail("not_actionable");
            sent = true;
            Element.prototype.scrollBy.call(scroll, {left:request.delta_x, top:request.delta_y, behavior:"instant"});
        } else return fail("unsupported");
        return {ok:true};
    } catch (_) {
        return fail(sent ? "interrupted" : "not_actionable");
    }
}
