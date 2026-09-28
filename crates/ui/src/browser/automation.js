(() => {
    const request = __REQUEST__;
    const previous = globalThis.__zeronAutomation;
    if (request.action === 'state') {
        const token = request.snapshot_id;
        const elements = new Map();
        const rows = [];
        for (const el of document.querySelectorAll('a,button,input,textarea,select,[role],[contenteditable="true"]')) {
            if (rows.length >= 200) break;
            const r = el.getBoundingClientRect();
            if (!r.width || !r.height || getComputedStyle(el).visibility === 'hidden') continue;
            const ref = token + ':' + rows.length;
            elements.set(ref, el);
            rows.push({ref, tag:el.tagName.toLowerCase(), role:el.getAttribute('role'),
                label:(el.getAttribute('aria-label') || el.labels?.[0]?.innerText || el.innerText || el.getAttribute('placeholder') || '').slice(0,200),
                type:el.type, value:el.type === 'password' ? '[redacted]' : String(el.value ?? '').slice(0,500),
                disabled:!!el.disabled, checked:el.checked});
        }
        globalThis.__zeronAutomation = {elements, url:location.href};
        return JSON.stringify({url:location.href,title:document.title,text:(document.body?.innerText || '').slice(0,24000),elements:rows,
            scope:'Main-frame light DOM; embedded frames and shadow DOM may require native inspection.'});
    }
    if (request.action === 'scroll') {
        window.scrollBy(0, request.delta_y);
        return JSON.stringify({effect:'dispatched-unknown',note:'Read state to verify the result.'});
    }
    if (previous?.url !== location.href) throw new Error("Navigation invalidated refs; read state again.");
    const el = previous?.elements.get(request.ref);
    if (!el || !el.isConnected) throw new Error('Stale element ref; read state again.');
    if (el.disabled || el.getAttribute('aria-disabled') === 'true') throw new Error('Element is disabled.');
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height || getComputedStyle(el).visibility === 'hidden') throw new Error('Element is hidden.');
    if (request.action === 'click') {
        el.click();
    } else if (request.action === 'type') {
        if (el.readOnly) throw new Error('Element is read-only.');
        if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
            const prototype = el instanceof HTMLInputElement ? HTMLInputElement.prototype : el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLSelectElement.prototype;
            Object.getOwnPropertyDescriptor(prototype, 'value').set.call(el, request.text);
        } else if (el.isContentEditable) el.textContent = request.text;
        else throw new Error('Element is not writable.');
        el.dispatchEvent(new Event('input', {bubbles:true}));
        el.dispatchEvent(new Event('change', {bubbles:true}));
    } else throw new Error('Unsupported DOM action.');
    return JSON.stringify({effect:'dispatched-unknown',input_route:'dom_event',note:'DOM event sent; read state to verify application behavior.'});
})()
