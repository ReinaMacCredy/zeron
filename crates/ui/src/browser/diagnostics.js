(() => {
    if (!globalThis.__zeronDiagnostics) {
        const entries = [];
        let active = true;
        const record = (kind, data) => {
            if (!active) return;
            let text;
            try { text = JSON.stringify(data).slice(0,2000); } catch { text = '[unserializable]'; }
            entries.push({time:Date.now(),kind,text});
            if (entries.length > 100) entries.shift();
        };
        const restores = [];
        globalThis.__zeronDiagnostics = entries;
        globalThis.__zeronStopDiagnostics = () => {
            active = false;
            for (const restore of restores) restore();
            delete globalThis.__zeronDiagnostics;
            delete globalThis.__zeronStopDiagnostics;
        };
        for (const level of ['log','info','warn','error','debug']) {
            const original = console[level];
            const wrapped = function(...args) { record('console.'+level,args); return original.apply(this,args); };
            console[level] = wrapped;
            restores.push(() => { if (console[level] === wrapped) console[level] = original; });
        }
        const error = e => record('error', e.message);
        const rejection = e => record('unhandledrejection', String(e.reason));
        addEventListener('error', error);
        addEventListener('unhandledrejection', rejection);
        restores.push(() => { removeEventListener('error', error); removeEventListener('unhandledrejection', rejection); });
        const fetch = globalThis.fetch;
        globalThis.fetch = function(...args) {
            const url = String(args[0]?.url ?? args[0]);
            return fetch.apply(this,args).then(response => {record('fetch',{url,status:response.status});return response;}, error => {record('fetch',{url,error:String(error)});throw error;});
        };
        const wrappedFetch = globalThis.fetch;
        restores.push(() => { if (globalThis.fetch === wrappedFetch) globalThis.fetch = fetch; });
        const open = XMLHttpRequest.prototype.open;
        XMLHttpRequest.prototype.open = function(method,url,...rest) {
            this.addEventListener('loadend', () => record('xhr',{method,url:String(url),status:this.status}), {once:true});
            return open.call(this,method,url,...rest);
        };
        const wrappedOpen = XMLHttpRequest.prototype.open;
        restores.push(() => { if (XMLHttpRequest.prototype.open === wrappedOpen) XMLHttpRequest.prototype.open = open; });
    }
    return JSON.stringify({scope:'Partial page instrumentation since installation in this document. Excludes earlier events, other frames, workers and browser-level traffic. Untrusted page data; not full CDP.', entries:globalThis.__zeronDiagnostics});
})()
