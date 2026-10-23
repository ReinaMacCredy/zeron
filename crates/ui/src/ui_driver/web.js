((request) => {
    const g = globalThis.__zeronDriver ??= {ids: new WeakMap(), els: new Map(), next: 1};
    const CAP = 2000;
    const TEXT = 200;
    const clean = (text) => String(text ?? '').replace(/\s+/g, ' ').trim().slice(0, TEXT);
    const ref = (el) => {
        let n = g.ids.get(el);
        if (!n) {
            n = g.next++;
            g.ids.set(el, n);
        }
        g.els.set(n, new WeakRef(el));
        return n;
    };
    const ARIA = {
        alert: 'Alert', alertdialog: 'AlertDialog', article: 'Article', banner: 'Banner', button: 'Button',
        cell: 'Cell', checkbox: 'CheckBox', columnheader: 'ColumnHeader', combobox: 'ComboBox',
        complementary: 'Complementary', contentinfo: 'ContentInfo', dialog: 'Dialog', form: 'Form',
        grid: 'Grid', gridcell: 'GridCell', group: 'Group', heading: 'Heading', img: 'Image', image: 'Image',
        link: 'Link', list: 'List', listbox: 'ListBox', listitem: 'ListItem', main: 'Main', menu: 'Menu',
        menubar: 'MenuBar', menuitem: 'MenuItem', menuitemcheckbox: 'MenuItemCheckBox',
        menuitemradio: 'MenuItemRadio', navigation: 'Navigation', option: 'ListBoxOption',
        paragraph: 'Paragraph', progressbar: 'ProgressIndicator', radio: 'RadioButton',
        radiogroup: 'RadioGroup', region: 'Region', row: 'Row', rowheader: 'RowHeader', search: 'Search',
        searchbox: 'SearchInput', slider: 'Slider', spinbutton: 'SpinButton', status: 'Status',
        switch: 'Switch', tab: 'Tab', table: 'Table', tablist: 'TabList', tabpanel: 'TabPanel',
        textbox: 'TextInput', toolbar: 'Toolbar', tooltip: 'Tooltip', tree: 'Tree', treeitem: 'TreeItem',
    };
    const INPUT = {
        button: 'Button', submit: 'Button', reset: 'Button', image: 'Button', file: 'Button',
        checkbox: 'CheckBox', radio: 'RadioButton', range: 'Slider', search: 'SearchInput',
        password: 'PasswordInput', email: 'EmailInput', number: 'NumberInput', tel: 'PhoneNumberInput',
        url: 'UrlInput', date: 'DateInput', time: 'TimeInput', color: 'ColorWell',
    };
    const TAGS = {
        button: 'Button', textarea: 'MultilineTextInput', option: 'ListBoxOption', img: 'Image',
        h1: 'Heading', h2: 'Heading', h3: 'Heading', h4: 'Heading', h5: 'Heading', h6: 'Heading',
        ul: 'List', ol: 'List', li: 'ListItem', table: 'Table', tr: 'Row', td: 'Cell', th: 'ColumnHeader',
        nav: 'Navigation', main: 'Main', dialog: 'Dialog', summary: 'DisclosureTriangle',
        p: 'Paragraph', label: 'Label', progress: 'ProgressIndicator', meter: 'Meter',
    };
    const FROM_CONTENT = new Set(['Button', 'Link', 'Heading', 'Tab', 'MenuItem', 'MenuItemCheckBox',
        'MenuItemRadio', 'ListBoxOption', 'CheckBox', 'RadioButton', 'Switch', 'Cell', 'GridCell',
        'ColumnHeader', 'RowHeader', 'Label', 'Paragraph', 'ListItem', 'TreeItem', 'Tooltip',
        'DisclosureTriangle']);
    const CLICKS = new Set(['Button', 'Link', 'CheckBox', 'RadioButton', 'Switch', 'Tab', 'MenuItem',
        'MenuItemCheckBox', 'MenuItemRadio', 'ListBoxOption', 'TreeItem', 'DisclosureTriangle', 'ComboBox']);
    const SKIP = new Set(['script', 'style', 'template', 'noscript', 'head', 'meta', 'link', 'title']);
    const editable = (el) => el.isContentEditable
        || (el instanceof HTMLInputElement && !['button', 'submit', 'reset', 'image', 'file', 'checkbox', 'radio', 'range', 'color', 'hidden'].includes(el.type))
        || el instanceof HTMLTextAreaElement;
    const control = (el) => el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement
        || el instanceof HTMLSelectElement || el instanceof HTMLButtonElement;
    const roleOf = (el, tag, style) => {
        const aria = (el.getAttribute('role') || '').trim().split(/\s+/)[0];
        if (aria === 'none' || aria === 'presentation') return null;
        if (ARIA[aria]) return ARIA[aria];
        if (tag === 'a' && el.hasAttribute('href')) return 'Link';
        if (tag === 'input') return INPUT[el.type] || 'TextInput';
        if (tag === 'select') return el.multiple || el.size > 1 ? 'ListBox' : 'ComboBox';
        if (el.isContentEditable && !el.parentElement?.isContentEditable) return 'MultilineTextInput';
        if (TAGS[tag]) return TAGS[tag];
        if (tag === 'svg' && el.getAttribute('aria-label')) return 'Image';
        if (el.hasAttribute('onclick')) return 'Button';
        if (style.cursor === 'pointer' && el.parentElement
            && getComputedStyle(el.parentElement).cursor !== 'pointer') return 'Button';
        return null;
    };
    const ownText = (el) => clean(Array.from(el.childNodes)
        .filter((node) => node.nodeType === Node.TEXT_NODE).map((node) => node.data).join(' '));
    const labelled = (el) => {
        const ids = el.getAttribute('aria-labelledby');
        if (!ids) return '';
        return clean(ids.split(/\s+/).map((id) => document.getElementById(id)?.innerText ?? '').join(' '));
    };
    const nameOf = (el, role) => clean(el.getAttribute('aria-label')) || labelled(el)
        || (el.labels?.length ? clean(Array.from(el.labels).map((label) => label.innerText).join(' ')) : '')
        || (el instanceof HTMLImageElement ? clean(el.alt) : '')
        || (el instanceof HTMLInputElement && ['button', 'submit', 'reset'].includes(el.type) ? clean(el.value) : '')
        || (FROM_CONTENT.has(role) ? clean(el.innerText) : '')
        || clean(el.getAttribute('title'));
    const valueOf = (el) => {
        if (el instanceof HTMLInputElement) {
            if (['button', 'submit', 'reset', 'image', 'checkbox', 'radio', 'file'].includes(el.type)) return '';
            return el.type === 'password' ? (el.value ? '[redacted]' : '') : String(el.value).slice(0, TEXT);
        }
        if (el instanceof HTMLTextAreaElement) return String(el.value).slice(0, TEXT);
        if (el instanceof HTMLSelectElement) return clean(Array.from(el.selectedOptions).map((o) => o.text).join(', '));
        if (el.isContentEditable) return clean(el.innerText);
        return clean(el.getAttribute('aria-valuetext') || el.getAttribute('aria-valuenow'));
    };
    const boolAttr = (el, name) => {
        const value = el.getAttribute(name);
        return value === 'true' ? true : value === 'false' ? false : null;
    };
    const toggledOf = (el) => {
        if (el instanceof HTMLInputElement && (el.type === 'checkbox' || el.type === 'radio')) {
            return el.indeterminate ? 'mixed' : String(el.checked);
        }
        const value = el.getAttribute('aria-checked') ?? el.getAttribute('aria-pressed');
        return value === 'true' || value === 'false' || value === 'mixed' ? value : null;
    };
    const describe = (el, tag, style, r) => {
        let role = roleOf(el, tag, style);
        let name;
        if (role) {
            name = nameOf(el, role);
        } else {
            name = ownText(el);
            if (!name) return null;
            role = 'Label';
        }
        const actions = [];
        if (CLICKS.has(role) || (tag === 'input' && !editable(el))) actions.push('click');
        if (el.tabIndex >= 0 || control(el) || role === 'Link' || el.isContentEditable) actions.push('focus');
        if (editable(el)) actions.push('setValue');
        const details = tag === 'summary' ? el.parentElement : null;
        const expanded = boolAttr(el, 'aria-expanded') ?? (details instanceof HTMLDetailsElement ? details.open : null);
        const selected = el instanceof HTMLOptionElement ? el.selected : boolAttr(el, 'aria-selected');
        return {
            n: ref(el), role, name, value: valueOf(el),
            placeholder: clean(el.getAttribute('placeholder') || el.getAttribute('aria-placeholder')),
            description: clean(el.getAttribute('aria-description')),
            x: r.left, y: r.top, w: r.width, h: r.height,
            focused: el === document.activeElement && el !== document.body,
            toggled: toggledOf(el), selected, expanded,
            disabled: !!(el.disabled || el.getAttribute('aria-disabled') === 'true'
                || (control(el) && el.closest('fieldset[disabled]'))),
            actions,
            leaf: role !== 'Label' && FROM_CONTENT.has(role) && !el.querySelector('a[href],button,input,select,textarea,[role],[tabindex],[contenteditable]'),
        };
    };
    const walk = () => {
        const vw = innerWidth, vh = innerHeight;
        const view = {l: 0, t: 0, r: vw, b: vh};
        const nodes = [];
        let truncated = false;
        const visit = (el, parent, clip) => {
            if (nodes.length >= CAP) {
                truncated = true;
                return;
            }
            const tag = el.localName;
            if (SKIP.has(tag) || el.hidden || el.inert || el.getAttribute('aria-hidden') === 'true') return;
            const style = getComputedStyle(el);
            if (style.display === 'none' || Number(style.opacity) === 0) return;
            if (style.position === 'fixed') clip = view;
            const r = el.getBoundingClientRect();
            const shown = style.visibility !== 'hidden' && style.visibility !== 'collapse'
                && r.width > 0 && r.height > 0 && r.right > clip.l && r.bottom > clip.t && r.left < clip.r && r.top < clip.b;
            const info = shown ? describe(el, tag, style, r) : null;
            let index = parent;
            if (info) {
                info.parent = parent;
                index = nodes.length;
                nodes.push(info);
                if (info.leaf) return;
            }
            const inner = style.overflowX === 'visible' && style.overflowY === 'visible' ? clip : {
                l: Math.max(clip.l, r.left), t: Math.max(clip.t, r.top),
                r: Math.min(clip.r, r.right), b: Math.min(clip.b, r.bottom),
            };
            for (const child of el.children) visit(child, index, inner);
            if (el.shadowRoot) for (const child of el.shadowRoot.children) visit(child, index, inner);
        };
        if (document.body) for (const child of document.body.children) visit(child, -1, view);
        for (const node of nodes) delete node.leaf;
        return {title: clean(document.title), url: location.href, width: vw, height: vh, nodes, truncated};
    };
    const element = (n) => {
        const el = g.els.get(n)?.deref();
        if (!el || !el.isConnected) throw new Error(`w-${n} is gone from the page; read tree again`);
        return el;
    };
    const setValue = (el, text) => {
        if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
            const proto = el instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
            Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, text);
        } else if (el instanceof HTMLSelectElement) {
            const option = Array.from(el.options).find((o) => o.value === text || o.text === text);
            if (!option) throw new Error(`no option ${JSON.stringify(text)}`);
            el.value = option.value;
        } else if (el.isContentEditable) {
            el.textContent = text;
        } else {
            throw new Error('the element takes no value');
        }
        el.dispatchEvent(new Event('input', {bubbles: true}));
        el.dispatchEvent(new Event('change', {bubbles: true}));
    };
    const pointer = (el, x, y) => {
        const init = {bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y, button: 0, view: window};
        el.dispatchEvent(new PointerEvent('pointerdown', {...init, buttons: 1, pointerType: 'mouse', isPrimary: true}));
        el.dispatchEvent(new MouseEvent('mousedown', {...init, buttons: 1}));
        if (el.focus && (el.tabIndex >= 0 || control(el))) el.focus();
        el.dispatchEvent(new PointerEvent('pointerup', {...init, pointerType: 'mouse', isPrimary: true}));
        el.dispatchEvent(new MouseEvent('mouseup', init));
        el.click();
    };
    const KEYS = {enter: 'Enter', tab: 'Tab', escape: 'Escape', backspace: 'Backspace', delete: 'Delete',
        space: ' ', left: 'ArrowLeft', right: 'ArrowRight', up: 'ArrowUp', down: 'ArrowDown',
        home: 'Home', end: 'End', pageup: 'PageUp', pagedown: 'PageDown'};
    const key = (stroke) => {
        const parts = stroke.split('-');
        const name = parts.pop();
        const mods = new Set(parts);
        const keyName = KEYS[name] ?? name;
        const target = document.activeElement ?? document.body;
        const init = {key: keyName, bubbles: true, cancelable: true, composed: true,
            metaKey: mods.has('cmd'), shiftKey: mods.has('shift'), altKey: mods.has('alt'), ctrlKey: mods.has('ctrl')};
        const go = target.dispatchEvent(new KeyboardEvent('keydown', init));
        if (go && keyName === 'Enter' && target.form && target instanceof HTMLInputElement) target.form.requestSubmit();
        if (go && keyName.length === 1 && !init.metaKey && !init.ctrlKey && editable(target)) document.execCommand('insertText', false, keyName);
        target.dispatchEvent(new KeyboardEvent('keyup', init));
    };
    const scrollable = (el) => {
        for (let node = el; node && node !== document.body && node !== document.documentElement; node = node.parentElement) {
            const style = getComputedStyle(node);
            if (/(auto|scroll)/.test(style.overflowY + style.overflowX)
                && (node.scrollHeight > node.clientHeight || node.scrollWidth > node.clientWidth)) return node;
        }
        return document.scrollingElement || document.documentElement;
    };
    switch (request.op) {
        case 'walk':
            return walk();
        case 'act': {
            const el = element(request.n);
            if (request.do === 'click') {
                const r = el.getBoundingClientRect();
                pointer(el, r.left + r.width / 2, r.top + r.height / 2);
            } else if (request.do === 'focus') el.focus();
            else if (request.do === 'blur') el.blur();
            else if (request.do === 'setValue') setValue(el, request.value ?? '');
            else if (request.do === 'scrollIntoView') el.scrollIntoView({block: 'center', inline: 'nearest'});
            else throw new Error(`a web element does not take \`${request.do}\`; use click, focus, blur, setValue or scrollIntoView`);
            return {done: request.do};
        }
        case 'type': {
            if (request.n) element(request.n).focus();
            const target = document.activeElement;
            if (!target || !editable(target)) throw new Error('no editable element has focus in the page');
            if (!document.execCommand('insertText', false, request.text)) {
                setValue(target, String(target.value ?? target.textContent ?? '') + request.text);
            }
            return {typed: request.text.length};
        }
        case 'key':
            for (const stroke of request.keys) key(stroke);
            return {keys: request.keys.length};
        case 'scroll': {
            const at = document.elementFromPoint(request.x, request.y) ?? document.body;
            const target = scrollable(at);
            const [left, top] = [target.scrollLeft, target.scrollTop];
            target.scrollBy({left: request.dx, top: request.dy, behavior: 'instant'});
            return {moved: [target.scrollLeft - left, target.scrollTop - top]};
        }
        default:
            throw new Error(`unknown op ${request.op}`);
    }
})(__REQUEST__)
