(()=>{var de=null,gr=()=>{if(de!==null)return de;try{de=window.matchMedia("(color-gamut: p3)").matches}catch{de=!1}return de},br=gr(),Z=(e)=>br?`color(display-p3 0.84 0.19 0.78 / ${e})`:`rgba(210, 57, 192, ${e})`;var nn=1e4;var vi=Z(0.4),xi=Z(0.05),Ei=Z(0.5),ki=Z(0.08),Si=Z(0.3),Ni=Z(0.04),Ai=Z(0.15);var zt="data-react-grab-frozen";var ot=new Set("display.position.top.right.bottom.left.z-index.overflow.overflow-x.overflow-y.width.height.min-width.min-height.max-width.max-height.margin-top.margin-right.margin-bottom.margin-left.padding-top.padding-right.padding-bottom.padding-left.flex-direction.flex-wrap.justify-content.align-items.align-self.align-content.flex-grow.flex-shrink.flex-basis.order.gap.row-gap.column-gap.grid-template-columns.grid-template-rows.grid-template-areas.font-family.font-size.font-weight.font-style.line-height.letter-spacing.text-align.text-decoration-line.text-decoration-style.text-transform.text-overflow.text-shadow.white-space.word-break.overflow-wrap.vertical-align.color.background-color.background-image.background-position.background-size.background-repeat.border-top-width.border-right-width.border-bottom-width.border-left-width.border-top-style.border-right-style.border-bottom-style.border-left-style.border-top-color.border-right-color.border-bottom-color.border-left-color.border-top-left-radius.border-top-right-radius.border-bottom-left-radius.border-bottom-right-radius.box-shadow.opacity.transform.filter.backdrop-filter.object-fit.object-position".split(".")),Dt=(e)=>(e.tagName||"").toLowerCase();var rn=typeof window<"u",wr=(e)=>0,yr=(e)=>{},Ht=rn?(Object.getOwnPropertyDescriptor(Window.prototype,"requestAnimationFrame")?.value??window.requestAnimationFrame).bind(window):wr,It=rn?(Object.getOwnPropertyDescriptor(Window.prototype,"cancelAnimationFrame")?.value??window.cancelAnimationFrame).bind(window):yr;var on="bippy-0.5.39",jt=Object.defineProperty,vr=Object.prototype.hasOwnProperty,he=()=>{},an=(e)=>{try{Function.prototype.toString.call(e).indexOf("^_^")>-1&&setTimeout(()=>{throw Error("React is running in production mode, but dead code elimination has not been applied. Read how to correctly configure React for production: https://reactjs.org/link/perf-use-production-build")})}catch{}},ln=(e=globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__)=>!!(e&&("getFiberRoots"in e)),sn=!1,Ft,et=(e=globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__)=>sn?!0:(e&&typeof e.inject=="function"&&(Ft=e.inject.toString()),!!Ft?.includes("(injected)")),Te=new Set,ge=new Set,xr=(e)=>{let t=new Map,n=0,r={_instrumentationIsActive:!1,_instrumentationSource:on,checkDCE:an,hasUnsupportedRendererAttached:!1,inject(o){let i=++n;return t.set(i,o),ge.add(o),r._instrumentationIsActive||(r._instrumentationIsActive=!0,Te.forEach((s)=>s())),i},on:he,onCommitFiberRoot:he,onCommitFiberUnmount:he,onPostCommitFiberRoot:he,renderers:t,supportsFiber:!0,supportsFlight:!0};try{jt(globalThis,"__REACT_DEVTOOLS_GLOBAL_HOOK__",{configurable:!0,enumerable:!0,get(){return r},set(s){if(s&&typeof s=="object"){let u=r.renderers;r=s,u.size>0&&(u.forEach((g,b)=>{ge.add(g),s.renderers.set(b,g)}),tt(e))}}});let o=window.hasOwnProperty,i=!1;jt(window,"hasOwnProperty",{configurable:!0,value:function(...s){try{if(!i&&s[0]==="__REACT_DEVTOOLS_GLOBAL_HOOK__")return globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__=void 0,i=!0,-0}catch{}return o.apply(this,s)},writable:!0})}catch{tt(e)}return r},tt=(e)=>{e&&Te.add(e);try{let t=globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__;if(!t)return;if(!t._instrumentationSource){t.checkDCE=an,t.supportsFiber=!0,t.supportsFlight=!0,t.hasUnsupportedRendererAttached=!1,t._instrumentationSource=on,t._instrumentationIsActive=!1;let n=ln(t);if(n||(t.on=he),t.renderers.size){t._instrumentationIsActive=!0,Te.forEach((i)=>i());return}let r=t.inject,o=et(t);o&&!n&&(sn=!0,t.inject({scheduleRefresh(){}})&&(t._instrumentationIsActive=!0)),t.inject=(i)=>{let s=r(i);return ge.add(i),o&&t.renderers.set(s,i),t._instrumentationIsActive=!0,Te.forEach((u)=>u()),s}}(t.renderers.size||t._instrumentationIsActive||et())&&e?.()}catch{}},Er=()=>vr.call(globalThis,"__REACT_DEVTOOLS_GLOBAL_HOOK__"),un=(e)=>Er()?(tt(e),globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__):xr(e),kr=()=>!!(typeof window<"u"&&(window.document?.createElement||window.navigator?.product==="ReactNative")),Sr=()=>{try{kr()&&un()}catch{}},cn=(e)=>{switch(e.tag){case 1:case 11:case 0:case 14:case 15:return!0;default:return!1}};function it(e,t,n=!1){if(!e)return null;let r=t(e);if(r instanceof Promise)return(async()=>{if(await r===!0)return e;let i=n?e.return:e.child;for(;i;){let s=await pn(i,t,n);if(s)return s;i=n?null:i.sibling}return null})();if(r===!0)return e;let o=n?e.return:e.child;for(;o;){let i=dn(o,t,n);if(i)return i;o=n?null:o.sibling}return null}var dn=(e,t,n=!1)=>{if(!e)return null;if(t(e)===!0)return e;let r=n?e.return:e.child;for(;r;){let o=dn(r,t,n);if(o)return o;r=n?null:r.sibling}return null},pn=async(e,t,n=!1)=>{if(!e)return null;if(await t(e)===!0)return e;let r=n?e.return:e.child;for(;r;){let o=await pn(r,t,n);if(o)return o;r=n?null:r.sibling}return null},fn=(e)=>{let t=e;return typeof t=="function"?t:typeof t=="object"&&t?fn(t.type||t.render):null},be=(e)=>{let t=e;if(typeof t=="string")return t;if(typeof t!="function"&&!(typeof t=="object"&&t))return null;let n=t.displayName||t.name||null;if(n)return n;let r=fn(t);return r&&(r.displayName||r.name)||null},_e=()=>{let e=globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__;return!!e?._instrumentationIsActive||ln(e)||et(e)};var re=(e)=>{let t=globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__;if(t?.renderers)for(let n of t.renderers.values())try{let r=n.findFiberByHostInstance?.(e);if(r)return r}catch{}if(typeof e=="object"&&e){if("_reactRootContainer"in e)return e._reactRootContainer?._internalRoot?.current?.child;for(let n in e)if(n.startsWith("__reactContainer$")||n.startsWith("__reactInternalInstance$")||n.startsWith("__reactFiber"))return e[n]||null}return null};var Vt=/^[a-zA-Z][a-zA-Z\d+\-.]*:/,Nr=["rsc://","file:///","webpack-internal://","webpack://","node:","turbopack://","metro://","/app-pages-browser/","/(app-pages-browser)/"],Ar=["<anonymous>","eval",""],mn=/\.(jsx|tsx|ts|js)$/,Cr=/(\.min|bundle|chunk|vendor|vendors|runtime|polyfill|polyfills)\.(js|mjs|cjs)$|(chunk|bundle|vendor|vendors|runtime|polyfill|polyfills|framework|app|main|index)[-_.][A-Za-z0-9_-]{4,}\.(js|mjs|cjs)$|[\da-f]{8,}\.(js|mjs|cjs)$|[-_.][\da-f]{20,}\.(js|mjs|cjs)$|\/dist\/|\/build\/|\/.next\/|\/out\/|\/node_modules\/|\.webpack\.|\.vite\.|\.turbopack\./i,Lr=/^\?[\w~.-]+(?:=[^&#]*)?(?:&[\w~.-]+(?:=[^&#]*)?)*$/,Tr=/(^|@)\S+:\d+/,hn=/^\s*at .*(\S+:\d+|\(native\))/m,Mr=/^(eval@)?(\[native code\])?$/,at=(e,t)=>{if(t?.includeInElement!==!1){let n=e.split(`
`),r=[];for(let o of n)if(/^\s*at\s+/.test(o)){let i=Bt(o,void 0)[0];i&&r.push(i)}else if(/^\s*in\s+/.test(o)){let i=o.replace(/^\s*in\s+/,"").replace(/\s*\(at .*\)$/,"");r.push({functionName:i,source:o})}else if(o.match(Tr)){let i=Wt(o,void 0)[0];i&&r.push(i)}return lt(r,t)}return e.match(hn)?Bt(e,t):Wt(e,t)},gn=(e)=>{if(!e.includes(":"))return[e,void 0,void 0];let t=e.startsWith("(")&&/:\d+\)$/.test(e)?e.slice(1,-1):e,n=/(.+?)(?::(\d+))?(?::(\d+))?$/.exec(t);return n?[n[1],n[2]||void 0,n[3]||void 0]:[t,void 0,void 0]},lt=(e,t)=>t&&t.slice!=null?Array.isArray(t.slice)?e.slice(t.slice[0],t.slice[1]):e.slice(0,t.slice):e,Bt=(e,t)=>lt(e.split(`
`).filter((n)=>!!n.match(hn)),t).map((n)=>{let r=n;r.includes("(eval ")&&(r=r.replace(/eval code/g,"eval").replace(/(\(eval at [^()]*)|(,.*$)/g,""));let o=r.replace(/^\s+/,"").replace(/\(eval code/g,"(").replace(/^.*?\s+/,""),i=o.match(/ (\(.+\)$)/);o=i?o.replace(i[0],""):o;let s=gn(i?i[1]:o);return{functionName:i&&o||void 0,fileName:["eval","<anonymous>"].includes(s[0])?void 0:s[0],lineNumber:s[1]?+s[1]:void 0,columnNumber:s[2]?+s[2]:void 0,source:r}}),Wt=(e,t)=>lt(e.split(`
`).filter((n)=>!n.match(Mr)),t).map((n)=>{let r=n;if(r.includes(" > eval")&&(r=r.replace(/ line (\d+)(?: > eval line \d+)* > eval:\d+:\d+/g,":$1")),!r.includes("@")&&!r.includes(":"))return{functionName:r};{let o=/(([^\n\r"\u2028\u2029]*".[^\n\r"\u2028\u2029]*"[^\n\r@\u2028\u2029]*(?:@[^\n\r"\u2028\u2029]*"[^\n\r@\u2028\u2029]*)*(?:[\n\r\u2028\u2029][^@]*)?)?[^@]*)@/,i=r.match(o),s=i&&i[1]?i[1]:void 0,u=gn(r.replace(o,""));return{functionName:s,fileName:u[0],lineNumber:u[1]?+u[1]:void 0,columnNumber:u[2]?+u[2]:void 0,source:r}}}),_r=44,Ut="ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",Rr=new Uint8Array(64),bn=new Uint8Array(128);for(let e=0;e<Ut.length;e++){let t=Ut.charCodeAt(e);Rr[e]=t,bn[t]=e}function pe(e,t){let n=0,r=0,o=0;do o=bn[e.next()],n|=(o&31)<<r,r+=5;while(o&32);let i=n&1;return n>>>=1,i&&(n=-2147483648|-n),t+n}function qt(e,t){return e.pos>=t?!1:e.peek()!==_r}var $r=class{constructor(e){this.pos=0,this.buffer=e}next(){return this.buffer.charCodeAt(this.pos++)}peek(){return this.buffer.charCodeAt(this.pos)}indexOf(e){let{buffer:t,pos:n}=this,r=t.indexOf(e,n);return r===-1?t.length:r}};function wn(e){let{length:t}=e,n=new $r(e),r=[],o=0,i=0,s=0,u=0,g=0;do{let b=n.indexOf(";"),d=[],f=!0,p=0;for(o=0;n.pos<b;){let h;o=pe(n,o),o<p&&(f=!1),p=o,qt(n,b)?(i=pe(n,i),s=pe(n,s),u=pe(n,u),qt(n,b)?(g=pe(n,g),h=[o,i,s,u,g]):h=[o,i,s,u]):h=[o],d.push(h),n.pos++}f||Or(d),r.push(d),n.pos=b+1}while(n.pos<=t);return r}function Or(e){e.sort(Pr)}function Pr(e,t){return e[0]-t[0]}var yn=/^[a-zA-Z][a-zA-Z\d+\-.]*:/,zr=/^data:application\/json[^,]+base64,/,Dr=/(?:\/\/[@#][ \t]+sourceMappingURL=([^\s'"]+?)[ \t]*$)|(?:\/\*[@#][ \t]+sourceMappingURL=([^*]+?)[ \t]*(?:\*\/)[ \t]*$)/,vn=typeof WeakRef<"u",fe=new Map,Ce=new Map,Hr=(e)=>vn&&e instanceof WeakRef,Gt=(e,t,n,r)=>{if(n<0||n>=e.length)return null;let o=e[n];if(!o||o.length===0)return null;let i=null;for(let d of o)if(d[0]<=r)i=d;else break;if(!i||i.length<4)return null;let[,s,u,g]=i;if(s===void 0||u===void 0||g===void 0)return null;let b=t[s];return b?{columnNumber:g,fileName:b,lineNumber:u+1}:null},Ir=(e,t,n)=>{if(e.sections){let r=null;for(let s of e.sections)if(t>s.offset.line||t===s.offset.line&&n>=s.offset.column)r=s;else break;if(!r)return null;let o=t-r.offset.line,i=t===r.offset.line?n-r.offset.column:n;return Gt(r.map.mappings,r.map.sources,o,i)}return Gt(e.mappings,e.sources,t-1,n)},jr=(e,t)=>{let n=t.split(`
`),r;for(let i=n.length-1;i>=0&&!r;i--){let s=n[i].match(Dr);s&&(r=s[1]||s[2])}if(!r)return null;let o=yn.test(r);if(!(zr.test(r)||o||r.startsWith("/"))){let i=e.split("/");i[i.length-1]=r,r=i.join("/")}return r},Fr=(e)=>({file:e.file,mappings:wn(e.mappings),names:e.names,sourceRoot:e.sourceRoot,sources:e.sources,sourcesContent:e.sourcesContent,version:3}),Vr=(e)=>{let t=e.sections.map(({map:r,offset:o})=>({map:{...r,mappings:wn(r.mappings)},offset:o})),n=new Set;for(let r of t)for(let o of r.map.sources)n.add(o);return{file:e.file,mappings:[],names:[],sections:t,sourceRoot:void 0,sources:Array.from(n),sourcesContent:void 0,version:3}},Xt=(e)=>{if(!e)return!1;let t=e.trim();if(!t)return!1;let n=t.match(yn);if(!n)return!0;let r=n[0].toLowerCase();return r==="http:"||r==="https:"},Br=async(e,t=fetch)=>{if(!Xt(e))return null;let n;try{let o=await t(e);if(!o.ok)return null;n=await o.text()}catch{return null}if(!n)return null;let r=jr(e,n);if(!r||!Xt(r))return null;try{let o=await t(r);if(!o.ok)return null;let i=await o.json();return"sections"in i?Vr(i):Fr(i)}catch{return null}},Wr=async(e,t=!0,n)=>{if(t&&fe.has(e)){let i=fe.get(e);if(i==null)return null;if(Hr(i)){let s=i.deref();if(s)return s;fe.delete(e)}else return i}if(t&&Ce.has(e))return Ce.get(e);let r=Br(e,n);t&&Ce.set(e,r);let o=await r;return t&&Ce.delete(e),t&&(o===null?fe.set(e,null):fe.set(e,vn?new WeakRef(o):o)),o},Ur=async(e,t=!0,n)=>await Promise.all(e.map(async(r)=>{if(!r.fileName)return r;let o=await Wr(r.fileName,t,n);if(!o||typeof r.lineNumber!="number"||typeof r.columnNumber!="number")return r;let i=Ir(o,r.lineNumber,r.columnNumber);return i?{...r,source:i.fileName&&r.source?r.source.replace(r.fileName,i.fileName):r.source,fileName:i.fileName,lineNumber:i.lineNumber,columnNumber:i.columnNumber,isSymbolicated:!0}:r})),xn=(e)=>e._debugStack instanceof Error&&typeof e._debugStack?.stack=="string",qr=()=>{let e=un();for(let t of[...Array.from(ge),...Array.from(e.renderers.values())]){let n=t.currentDispatcherRef;if(n&&typeof n=="object")return"H"in n?n.H:n.current}return null},Yt=(e)=>{for(let t of ge){let n=t.currentDispatcherRef;n&&typeof n=="object"&&("H"in n?n.H=e:n.current=e)}},I=(e)=>`
    in ${e}`,Gr=(e,t)=>{let n=I(e);return t&&(n+=` (at ${t})`),n},Ke=!1,Ze=(e,t)=>{if(!e||Ke)return"";let n=Error.prepareStackTrace;Error.prepareStackTrace=void 0,Ke=!0;let r=qr();Yt(null);let{error:o,warn:i}=console;console.error=()=>{},console.warn=()=>{};try{let u={DetermineComponentFrameRoot(){let d;try{if(t){let f=function(){throw Error()};if(Object.defineProperty(f.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(f,[])}catch(p){d=p}Reflect.construct(e,[],f)}else{try{f.call()}catch(p){d=p}e.call(f.prototype)}}else{try{throw Error()}catch(p){d=p}let f=e();f&&typeof f.catch=="function"&&f.catch(()=>{})}}catch(f){if(f instanceof Error&&d instanceof Error&&typeof f.stack=="string")return[f.stack,d.stack]}return[null,null]}};u.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot",Object.getOwnPropertyDescriptor(u.DetermineComponentFrameRoot,"name")?.configurable&&Object.defineProperty(u.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});let[g,b]=u.DetermineComponentFrameRoot();if(g&&b){let d=g.split(`
`),f=b.split(`
`),p=0,h=0;for(;p<d.length&&!d[p].includes("DetermineComponentFrameRoot");)p++;for(;h<f.length&&!f[h].includes("DetermineComponentFrameRoot");)h++;if(p===d.length||h===f.length)for(p=d.length-1,h=f.length-1;p>=1&&h>=0&&d[p]!==f[h];)h--;for(;p>=1&&h>=0;p--,h--)if(d[p]!==f[h]){if(p!==1||h!==1)do if(p--,h--,h<0||d[p]!==f[h]){let O=`
${d[p].replace(" at new "," at ")}`,A=be(e);return A&&O.includes("<anonymous>")&&(O=O.replace("<anonymous>",A)),O}while(p>=1&&h>=0);break}}}finally{Ke=!1,Error.prepareStackTrace=n,Yt(r),console.error=o,console.warn=i}let s=e?be(e):"";return s?I(s):""},Xr=(e,t)=>{let n=e.tag,r="";switch(n){case 28:r=I("Activity");break;case 1:r=Ze(e.type,!0);break;case 11:r=Ze(e.type.render,!1);break;case 0:case 15:r=Ze(e.type,!1);break;case 5:case 26:case 27:r=I(e.type);break;case 16:r=I("Lazy");break;case 13:r=e.child!==t&&t!==null?I("Suspense Fallback"):I("Suspense");break;case 19:r=I("SuspenseList");break;case 30:r=I("ViewTransition");break;default:return""}return r},Yr=(e)=>{try{let t="",n=e,r=null;do{t+=Xr(n,r);let o=n._debugInfo;if(o&&Array.isArray(o))for(let i=o.length-1;i>=0;i--){let s=o[i];typeof s.name=="string"&&(t+=Gr(s.name,s.env))}r=n,n=n.return}while(n);return t}catch(t){return t instanceof Error?`
Error generating stack: ${t.message}
${t.stack}`:""}},En=(e)=>{let t=Error.prepareStackTrace;Error.prepareStackTrace=void 0;let n=e;if(!n)return"";Error.prepareStackTrace=t,n.startsWith(`Error: react-stack-top-frame
`)&&(n=n.slice(29));let r=n.indexOf(`
`);if(r!==-1&&(n=n.slice(r+1)),r=Math.max(n.indexOf("react_stack_bottom_frame"),n.indexOf("react-stack-bottom-frame")),r!==-1&&(r=n.lastIndexOf(`
`,r)),r!==-1)n=n.slice(0,r);else return"";return n},Kr=(e)=>!!(e.fileName?.startsWith("rsc://")&&e.functionName),Zr=(e,t)=>e.fileName===t.fileName&&e.lineNumber===t.lineNumber&&e.columnNumber===t.columnNumber,Jr=(e)=>{let t=new Map;for(let n of e)for(let r of n.stackFrames){if(!Kr(r))continue;let o=r.functionName,i=t.get(o)??[];i.some((s)=>Zr(s,r))||(i.push(r),t.set(o,i))}return t},Qr=(e,t,n)=>{if(!e.functionName)return{...e,isServer:!0};let r=t.get(e.functionName);if(!r||r.length===0)return{...e,isServer:!0};let o=n.get(e.functionName)??0,i=r[o%r.length];return n.set(e.functionName,o+1),{...e,isServer:!0,fileName:i.fileName,lineNumber:i.lineNumber,columnNumber:i.columnNumber,source:e.source?.replace("(at Server)",`(${i.fileName}:${i.lineNumber}:${i.columnNumber})`)}},eo=(e)=>{let t=[];return it(e,(n)=>{if(!xn(n))return;let r=typeof n.type=="string"?n.type:be(n.type)||"<anonymous>";t.push({componentName:r,stackFrames:at(En(n._debugStack?.stack))})},!0),t},to=async(e,t=!0,n)=>{let r=eo(e),o=at(Yr(e)),i=Jr(r),s=new Map;return Ur(o.map((u)=>u.source?.includes("(at Server)")??!1?Qr(u,i,s):u).filter((u,g,b)=>{if(g===0)return!0;let d=b[g-1];return u.functionName!==d.functionName}),t,n)},Kt=(e)=>e.split("/").filter(Boolean).length,no=(e)=>e.split("/").filter(Boolean)[0]??null,ro=(e)=>{let t=e.indexOf("/",1);if(t===-1||Kt(e.slice(0,t))!==1)return e;let n=e.slice(t);if(!mn.test(n)||Kt(n)<2)return e;let r=no(n);return!r||r.startsWith("@")||r.length>4?e:n},kn=(e)=>{if(!e||Ar.some((i)=>i===e))return"";let t=e,n=t.startsWith("http://")||t.startsWith("https://");if(n)try{t=new URL(t).pathname}catch{}if(n&&(t=ro(t)),t.startsWith("about://React/")){let i=t.slice(14),s=i.indexOf("/"),u=i.indexOf(":");t=s!==-1&&(u===-1||s<u)?i.slice(s+1):i}let r=!0;for(;r;){r=!1;for(let i of Nr)if(t.startsWith(i)){t=t.slice(i.length),i==="file:///"&&(t=`/${t.replace(/^\/+/,"")}`),r=!0;break}}if(Vt.test(t)){let i=t.match(Vt);i&&(t=t.slice(i[0].length))}if(t.startsWith("//")){let i=t.indexOf("/",2);t=i===-1?"":t.slice(i)}let o=t.indexOf("?");if(o!==-1){let i=t.slice(o);Lr.test(i)&&(t=t.slice(0,o))}return t},Sn=(e)=>{let t=kn(e);return!(!t||!mn.test(t)||Cr.test(t))},oo=Symbol.for("react.context"),Nn=[],Zt=null,An=Error("Suspense Exception: This is not a real error! It's an implementation detail of `use` to interrupt the current render."),E=()=>{let e=Zt;return e!==null&&(Zt=e.next),e},Me=(e)=>e._currentValue,x=(e,t,n,r=null)=>{Nn.push({displayName:r,primitive:e,stackError:Error(),value:t,dispatcherHookName:n})},io=(e)=>{if(typeof e=="object"&&e){let t=e;if(typeof t.then=="function"){let n=t;switch(n.status){case"fulfilled":return x("Promise",n.value,"Use"),n.value;case"rejected":throw n.reason}throw x("Unresolved",n,"Use"),An}if(t.$$typeof===oo&&"_currentValue"in t){let n=t,r=Me(n);return x("Context (use)",r,"Use",n.displayName||"Context"),r}}throw Error("An unsupported type was passed to use(): "+String(e))},ao=(e)=>{let t=Me(e);return x("Context",t,"Context",e.displayName||null),t},lo=(e)=>{let t=E(),n=t===null?typeof e=="function"?e():e:t.memoizedState;return x("State",n,"State"),[n,()=>{}]},so=(e,t,n)=>{let r=E(),o=r===null?n===void 0?t:n(t):r.memoizedState;return x("Reducer",o,"Reducer"),[o,()=>{}]},uo=(e)=>{let t=E(),n=t===null?{current:e}:t.memoizedState;return x("Ref",n.current,"Ref"),n},co=()=>{let e=E();return x("CacheRefresh",e===null?()=>{}:e.memoizedState,"CacheRefresh"),()=>{}},po=(e)=>{E(),x("LayoutEffect",e,"LayoutEffect")},fo=(e)=>{E(),x("InsertionEffect",e,"InsertionEffect")},mo=(e)=>{E(),x("Effect",e,"Effect")},ho=(e)=>{E();let t;typeof e=="object"&&e&&"current"in e&&(t=e.current),x("ImperativeHandle",t,"ImperativeHandle")},go=(e,t)=>{x("DebugValue",typeof t=="function"?t(e):e,"DebugValue")},bo=(e)=>{let t=E();return x("Callback",t===null?e:t.memoizedState[0],"Callback"),e},wo=(e)=>{let t=E(),n=t===null?e():t.memoizedState[0];return x("Memo",n,"Memo"),n},yo=(e,t)=>{let n=E();E();let r=n===null?t():n.memoizedState;return x("SyncExternalStore",r,"SyncExternalStore"),r},vo=()=>{let e=E();E();let t=e===null?!1:e.memoizedState;return x("Transition",t,"Transition"),[t,()=>{}]},xo=(e)=>{let t=E(),n=t===null?e:t.memoizedState;return x("DeferredValue",n,"DeferredValue"),n},Eo=()=>{let e=E(),t=e===null?"":e.memoizedState;return x("Id",t,"Id"),t},ko=(e)=>[],So=(e)=>{let t=E(),n=t===null?e:t.memoizedState;return x("Optimistic",n,"Optimistic"),[n,()=>{}]},No=(e,t)=>{let n,r=null;if(e!==null){let o=e.memoizedState;if(typeof o=="object"&&o&&"then"in o&&typeof o.then=="function"){let i=o;switch(i.status){case"fulfilled":n=i.value;break;case"rejected":r=i.reason;break;default:r=An,n=i}}else n=o}else n=t;return{value:n,error:r}},Cn=(e)=>(t,n)=>{let r=E();E(),E();let o=Error(),{value:i,error:s}=No(r,n);if(Nn.push({displayName:null,primitive:e,stackError:o,value:i,dispatcherHookName:e}),s!==null)throw s;return[i,()=>{},!1]},Ao=Cn("ActionState"),Co={readContext:Me,use:io,useCallback:bo,useContext:ao,useEffect:mo,useImperativeHandle:ho,useLayoutEffect:po,useInsertionEffect:fo,useMemo:wo,useReducer:so,useRef:uo,useState:lo,useDebugValue:go,useDeferredValue:xo,useTransition:vo,useSyncExternalStore:yo,useId:Eo,useHostTransitionStatus:()=>{let e=Me({_currentValue:null});return x("HostTransitionStatus",e,"HostTransitionStatus"),e},useFormState:Cn("FormState"),useActionState:Ao,useOptimistic:So,useMemoCache:ko,useCacheRefresh:co,useEffectEvent:(e)=>(E(),x("EffectEvent",e,"EffectEvent"),e)};typeof Proxy>"u"||new Proxy(Co,{get(e,t){if(Object.prototype.hasOwnProperty.call(e,t))return e[t];let n=Error("Missing method in Dispatcher: "+t);throw n.name="ReactDebugToolsUnsupportedHookError",n}}),Sr();var Ln=(e,t)=>e.length>t?`${e.slice(0,t)}...`:e,Le,Lo=()=>{if(Le!==void 0)return Le;let e=document.querySelector('script[src*="/_next/"]')?.src,t=e?new URL(e).pathname:"",n=t.indexOf("/_next/");return Le=n>0?t.slice(0,n):"",Le},To=/^(?:\.\/)?\/?\([a-z][a-z0-9-]*\)\//,Mo=(e)=>{let t=kn(e);return t=t.replace(To,""),t.startsWith("./")&&(t=t.slice(2)),t};var _o=(e)=>e.startsWith("data-react-grab-");var Ro=new Set(["_","$","motion.","styled.","chakra.","ark.","Primitive.","Slot."]),$o=new Set(["InnerLayoutRouter","RedirectErrorBoundary","RedirectBoundary","HTTPAccessFallbackErrorBoundary","HTTPAccessFallbackBoundary","LoadingBoundary","ErrorBoundary","InnerScrollAndFocusHandler","ScrollAndFocusHandler","RenderFromTemplateContext","OuterLayoutRouter","body","html","DevRootHTTPAccessFallbackBoundary","AppDevOverlayErrorBoundary","AppDevOverlay","HotReload","Router","ErrorBoundaryHandler","AppRouter","ServerRoot","SegmentStateProvider","RootErrorBoundary","LoadableComponent","MotionDOMComponent"]),Oo=new Set(["Suspense","Fragment","StrictMode","Profiler","SuspenseList"]),Je,Tn=(e)=>(e&&(Je=void 0),Je??=typeof document<"u"&&!!(document.getElementById("__NEXT_DATA__")||document.querySelector("nextjs-portal")),Je),Mn=(e)=>{if($o.has(e)||Oo.has(e))return!0;for(let t of Ro)if(e.startsWith(t))return!0;return!1},_n=(e)=>!(!e||Mn(e)||e==="SlotClone"||e==="Slot"),nt=(e)=>!(e.length<=1||Mn(e)||e[0]!==e[0].toUpperCase()||e.endsWith("Provider")||e.endsWith("Context")),Rn=["about://React/","rsc://React/"],Po=(e)=>Rn.some((t)=>e.startsWith(t)),zo=(e)=>{for(let t of Rn){if(!e.startsWith(t))continue;let n=e.indexOf("/",t.length),r=e.lastIndexOf("?");if(n>-1&&r>-1)return decodeURI(e.slice(n+1,r))}return e},Do=async(e)=>{let t=[],n=[];for(let i=0;i<e.length;i++){let s=e[i];!s.isServer||!s.fileName||(t.push(i),n.push({file:zo(s.fileName),methodName:s.functionName??"<unknown>",line1:s.lineNumber??null,column1:s.columnNumber??null,arguments:[]}))}if(n.length===0)return e;let r=new AbortController,o=setTimeout(()=>r.abort(),5000);try{let i=await fetch(`${Lo()}/__nextjs_original-stack-frames`,{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({frames:n,isServer:!0,isEdgeServer:!1,isAppDirectory:!0}),signal:r.signal});if(!i.ok)return e;let s=await i.json(),u=[...e];for(let g=0;g<t.length;g++){let b=s[g];if(b?.status!=="fulfilled")continue;let d=b.value?.originalStackFrame;if(!d?.file||d.ignored)continue;let f=t[g];u[f]={...e[f],fileName:d.file,lineNumber:d.line1??void 0,columnNumber:d.column1??void 0,isSymbolicated:!0}}return u}catch{return e}finally{clearTimeout(o)}},Ho=(e)=>{let t=new Map;return it(e,(n)=>{if(!xn(n))return!1;let r=En(n._debugStack.stack);if(!r)return!1;for(let o of at(r))!o.functionName||!o.fileName||Po(o.fileName)&&(t.has(o.functionName)||t.set(o.functionName,{...o,isServer:!0}));return!1},!0),t},Io=(e,t)=>{if(!t.some((r)=>r.isServer&&!r.fileName&&r.functionName))return t;let n=Ho(e);return n.size===0?t:t.map((r)=>{if(!r.isServer||r.fileName||!r.functionName)return r;let o=n.get(r.functionName);return o?{...r,fileName:o.fileName,lineNumber:o.lineNumber,columnNumber:o.columnNumber}:r})},$n=(e)=>{if(!_e())return e;let t=e;for(;t;){if(re(t))return t;t=t.parentElement}return e},Jt=new WeakMap,jo=async(e)=>{try{let t=re(e);if(!t)return null;let n=await to(t);return Tn()?await Do(Io(t,n)):n}catch{return null}},st=(e)=>{if(!_e())return Promise.resolve([]);let t=$n(e),n=Jt.get(t);if(n)return n;let r=jo(t);return Jt.set(t,r),r};var On=(e)=>{if(!_e())return null;let t=re($n(e));if(!t)return null;let n=t.return;for(;n;){if(cn(n)){let r=be(n.type);if(r&&_n(r))return r}n=n.return}return null},Fo=(e)=>e?e.some((t)=>!!(t.fileName&&Sn(t.fileName)||t.isServer&&(!t.functionName||nt(t.functionName)))):!1,Vo=(e,t)=>{if(!_e())return[];let n=re(e);if(!n)return[];let r=[];return it(n,(o)=>{if(r.length>=t)return!0;if(cn(o)){let i=be(o.type);i&&_n(i)&&r.push(i)}return!1},!0),r},Bo=(e,t={})=>{let{maxLines:n=3}=t,r=Tn(),o=[];for(let i of e){if(o.length>=n)break;let s=i.fileName&&Sn(i.fileName);if(i.isServer&&!s&&(!i.functionName||nt(i.functionName))){o.push(`
  in ${i.functionName||"<anonymous>"} (at Server)`);continue}if(s){let u=`
  in `,g=i.functionName&&nt(i.functionName);g&&(u+=`${i.functionName} (at `),u+=Mo(i.fileName),r&&i.lineNumber&&(u+=`:${i.lineNumber}`,i.columnNumber&&(u+=`:${i.columnNumber}`)),g&&(u+=")"),o.push(u)}}return o.join("")},Pn=async(e,t={})=>{let n=t.maxLines??3,r=await st(e);if(r&&Fo(r))return Bo(r,t);let o=Vo(e,n);return o.length>0?o.map((i)=>`
  in ${i}`).join(""):""};var Wo=(e)=>Ln(e,15);var zn=(e)=>{let t=Dt(e),n=e instanceof HTMLElement?e.innerText?.trim()??e.textContent?.trim()??"":e.textContent?.trim()??"",r="";for(let{name:p,value:h}of e.attributes)_o(p)||(r+=` ${p}="${Wo(h)}"`);let o=[],i=[],s=!1,u=Array.from(e.childNodes);for(let p of u)p.nodeType!==Node.COMMENT_NODE&&(p.nodeType===Node.TEXT_NODE?p.textContent&&p.textContent.trim().length>0&&(s=!0):p instanceof Element&&(s?i.push(p):o.push(p)));let g=(p)=>p.length===0?"":p.length<=2?p.map((h)=>`<${Dt(h)} ...>`).join(`
  `):`(${p.length} elements)`,b="",d=g(o);d&&(b+=`
  ${d}`),n.length>0&&(b+=`
  ${Ln(n,100)}`);let f=g(i);return f&&(b+=`
  ${f}`),b.length>0?`<${t}${r}>${b}
</${t}>`:`<${t}${r} />`};var rt=!1,me=new Map,Qt=-1,en=new WeakSet,Qe=new Map,tn=new Map,Uo=(e)=>en.has(e)?!0:!rt||!("gsapVersions"in window)||!(Error().stack??"").includes("_tick")?!1:(en.add(e),!0);typeof window<"u"&&(window.requestAnimationFrame=(e)=>{if(!Uo(e))return Ht(e);if(rt){let n=Qt--;return me.set(n,e),n}let t=Ht((n)=>{if(rt){let r=Qt--;me.set(r,e),Qe.set(t,r);return}e(n)});return t},window.cancelAnimationFrame=(e)=>{if(me.has(e)){me.delete(e);return}let t=tn.get(e);if(t!==void 0){It(t.nativeId),tn.delete(e);return}let n=Qe.get(e);if(n!==void 0){me.delete(n),Qe.delete(e);return}It(e)});var Ci=`
[${zt}],
[${zt}] * {
  animation-play-state: paused !important;
  transition: none !important;
}
`;var qo=new Set(["role","name","aria-label","rel","href"]),ye=(e)=>{if(!/^[a-z\-]{3,}$/i.test(e))return!1;let t=e.split(/-|[A-Z]/);for(let n of t)if(n.length<=2||/[^aeiou]{4,}/i.test(n))return!1;return!0},Go=(e,t)=>{let n=qo.has(e)||e.startsWith("data-")&&ye(e),r=ye(t)&&t.length<100||t.startsWith("#")&&ye(t.slice(1));return n&&r},ct=(e)=>{let t=e[0].name;for(let n=1;n<e.length;n++)t=`${e[n].name} > ${t}`;return t},Dn=(e)=>{let t=0;for(let n of e)t+=n.penalty;return t},Hn=(e,t)=>Dn(e)-Dn(t),dt=(e,t)=>{let n=e.parentNode;if(!n)return;let r=n.firstChild;if(!r)return;let o=0;for(;r&&(r.nodeType===Node.ELEMENT_NODE&&(t===void 0||r.tagName.toLowerCase()===t)&&o++,r!==e);)r=r.nextSibling;return o},Xo=(e,t)=>e==="html"?"html":`${e}:nth-child(${t})`,jn=(e,t)=>e==="html"?"html":`${e}:nth-of-type(${t})`,Yo=(e,t)=>{let n=[],r=e.getAttribute("id"),o=e.tagName.toLowerCase();r&&ye(r)&&n.push({name:`#${CSS.escape(r)}`,penalty:0});for(let u of e.classList)ye(u)&&n.push({name:`.${CSS.escape(u)}`,penalty:1});for(let u of e.attributes)t(u.name,u.value)&&n.push({name:`[${CSS.escape(u.name)}="${CSS.escape(u.value)}"]`,penalty:2});n.push({name:o,penalty:5});let i=dt(e,o);i!==void 0&&n.push({name:jn(o,i),penalty:10});let s=dt(e);return s!==void 0&&n.push({name:Xo(o,s),penalty:50}),n},pt=(e,t=nn,n=[])=>{if(t<=0)return[];if(e.length===0)return[n];let r=[];for(let o of e[0]){let i=t-r.length;if(i<=0)break;r.push(...pt(e.slice(1),i,[...n,o]))}return r},Ko=(e,t)=>{let n=t.getRootNode?.();return n instanceof ShadowRoot?n:e.nodeType===Node.DOCUMENT_NODE?e:e.ownerDocument},ft=(e,t)=>t.querySelectorAll(ct(e)).length===1,Zo=(e,t)=>{let n=e,r=[];for(;n&&n!==t;){let o=n.tagName.toLowerCase(),i=dt(n,o);if(i===void 0)return;r.push({name:jn(o,i),penalty:10}),n=n.parentElement}return ft(r,t)?r:void 0},Jo=(e,t,n,r)=>{if(e.nodeType!==Node.ELEMENT_NODE)throw Error("Can't generate CSS selector for non-element node type.");if(e.tagName.toLowerCase()==="html")return"html";let o=Ko(t,e),i=Date.now(),s=[],u=e,g=0,b;for(;u&&u!==o&&!b;)if(s.push(Yo(u,r)),u=u.parentElement,g++,g>=3){let d=pt(s);d.sort(Hn);for(let f of d){if(Date.now()-i>n){let p=Zo(e,o);if(!p)throw Error(`Timeout: Can't find a unique selector after ${n}ms`);return ct(p)}if(ft(f,o)){b=f;break}}}if(!b&&g<3){let d=pt(s);d.sort(Hn);for(let f of d){if(Date.now()-i>n)break;if(ft(f,o)){b=f;break}}}if(!b)throw Error("Selector was not found.");return ct(b)},Fn=(e)=>typeof CSS<"u"&&typeof CSS.escape=="function"?CSS.escape(e):e.replace(/[^a-zA-Z0-9_-]/g,(t)=>`\\${t}`),Vn=(e)=>e.ownerDocument.body??e.ownerDocument.documentElement,Bn=new Set(["data-testid","data-test-id","data-test","data-cy","data-qa","aria-label","role","name","title","alt"]),Wn=(e)=>e.length>0&&e.length<=120,ut=(e,t)=>{try{let n=e.ownerDocument.querySelectorAll(t);return n.length===1&&n[0]===e}catch{return!1}},Qo=(e)=>{if(e instanceof HTMLElement&&e.id){let t=`#${Fn(e.id)}`;if(ut(e,t))return t}for(let t of Bn){let n=e.getAttribute(t);if(!n||!Wn(n))continue;let r=`[${t}=${JSON.stringify(n)}]`;if(ut(e,r))return r;let o=`${e.tagName.toLowerCase()}${r}`;if(ut(e,o))return o}return null},ei=(e)=>{let t=[],n=Vn(e),r=e;for(;r;){if(r instanceof HTMLElement&&r.id){t.unshift(`#${Fn(r.id)}`);break}let o=r.parentElement;if(!o){t.unshift(r.tagName.toLowerCase());break}let i=Array.from(o.children).indexOf(r),s=i>=0?i+1:1;if(t.unshift(`${r.tagName.toLowerCase()}:nth-child(${s})`),o===n){t.unshift(n.tagName.toLowerCase());break}r=o}return t.join(" > ")},Un=(e,t=!0)=>{let n=Qo(e);if(n)return n;if(t)try{let r=Jo(e,Vn(e),200,(o,i)=>Go(o,i)||Bn.has(o)&&Wn(i));if(r)return r}catch{}return ei(e)},ti=new Map(["top","right","bottom","left"].flatMap((e)=>[[`border-${e}-style`,e],[`border-${e}-color`,e]])),we=null,In=new Map,ni=()=>we||(we=document.createElement("iframe"),we.style.cssText="position:fixed;left:-9999px;width:0;height:0;border:none;visibility:hidden;",document.body.appendChild(we),we),ri=(e)=>{let t=In.get(e);if(t)return t;let n=ni(),r=n.contentDocument,o=r.createElement(e);r.body.appendChild(o);let i=n.contentWindow.getComputedStyle(o),s=new Map;for(let u of ot){let g=i.getPropertyValue(u);g&&s.set(u,g)}return o.remove(),In.set(e,s),s},oi=(e,t)=>{let n=ti.get(e);if(!n)return!1;let r=t.getPropertyValue(`border-${n}-width`);return r==="0px"||r==="0"},qn=(e)=>{let t=ri(e.tagName.toLowerCase()),n=getComputedStyle(e),r=[];for(let s of ot){let u=n.getPropertyValue(s);u&&u!==t.get(s)&&(oi(s,n)||r.push(`${s}: ${u};`))}let o=e.getAttribute("class")?.trim(),i=r.join(`
`);return o?i?`className: ${o}

${i}`:`className: ${o}`:i};var Gn=async(e)=>{let t=await st(e)??[],n=await Pn(e);return{element:e,htmlPreview:zn(e),stackString:n,stack:t,componentName:On(e),fiber:re(e),selector:Un(e),styles:qn(e)}};var Xn=`/*! tailwindcss v4.3.3 | MIT License | https://tailwindcss.com */
@layer properties;
:root, :host {
  --spacing: 0.25rem;
  --text-xs: 0.75rem;
  --text-xs--line-height: calc(1 / 0.75);
  --text-sm: 0.875rem;
  --text-sm--line-height: calc(1.25 / 0.875);
  --text-lg: 1.125rem;
  --text-lg--line-height: calc(1.75 / 1.125);
  --font-weight-medium: 500;
  --font-weight-semibold: 600;
  --font-weight-bold: 700;
  --blur-xl: 24px;
  --default-font-family: var(--t3-font-sans);
  --default-mono-font-family: var(--t3-font-mono);
}
*, ::after, ::before, ::backdrop, ::file-selector-button {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
  border: 0 solid;
}
html, :host {
  line-height: 1.5;
  -webkit-text-size-adjust: 100%;
  tab-size: 4;
  font-family: var(--default-font-family, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', 'Noto Sans', Arial, sans-serif, 'Apple Color Emoji', 'Segoe UI Emoji', 'Segoe UI Symbol', 'Noto Color Emoji');
  font-feature-settings: var(--default-font-feature-settings, normal);
  font-variation-settings: var(--default-font-variation-settings, normal);
  -webkit-tap-highlight-color: transparent;
}
hr {
  height: 0;
  color: inherit;
  border-top-width: 1px;
}
abbr:where([title]) {
  -webkit-text-decoration: underline dotted;
  text-decoration: underline dotted;
}
h1, h2, h3, h4, h5, h6 {
  font-size: inherit;
  font-weight: inherit;
}
a {
  color: inherit;
  -webkit-text-decoration: inherit;
  text-decoration: inherit;
}
b, strong {
  font-weight: bolder;
}
code, kbd, samp, pre {
  font-family: var(--default-mono-font-family, ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace);
  font-feature-settings: var(--default-mono-font-feature-settings, normal);
  font-variation-settings: var(--default-mono-font-variation-settings, normal);
  font-size: 1em;
}
small {
  font-size: 80%;
}
sub, sup {
  font-size: 75%;
  line-height: 0;
  position: relative;
  vertical-align: baseline;
}
sub {
  bottom: -0.25em;
}
sup {
  top: -0.5em;
}
table {
  text-indent: 0;
  border-color: inherit;
  border-collapse: collapse;
}
:-moz-focusring:where(:not(iframe)) {
  outline: auto;
}
progress {
  vertical-align: baseline;
}
summary {
  display: list-item;
}
ol, ul, menu {
  list-style: none;
}
img, svg, video, canvas, audio, iframe, embed, object {
  display: block;
  vertical-align: middle;
}
img, video {
  max-width: 100%;
  height: auto;
}
button, input, select, optgroup, textarea, ::file-selector-button {
  font: inherit;
  font-feature-settings: inherit;
  font-variation-settings: inherit;
  letter-spacing: inherit;
  color: inherit;
  border-radius: 0;
  background-color: transparent;
  opacity: 1;
}
:where(select:is([multiple], [size])) optgroup {
  font-weight: bolder;
}
:where(select:is([multiple], [size])) optgroup option {
  padding-inline-start: 20px;
}
::file-selector-button {
  margin-inline-end: 4px;
}
::placeholder {
  opacity: 1;
}
@supports (not (-webkit-appearance: -apple-pay-button))  or (contain-intrinsic-size: 1px) {
  ::placeholder {
    color: currentcolor;
    @supports (color: color-mix(in lab, red, red)) {
      color: color-mix(in oklab, currentcolor 50%, transparent);
    }
  }
}
textarea {
  resize: vertical;
}
::-webkit-search-decoration {
  -webkit-appearance: none;
}
::-webkit-date-and-time-value {
  min-height: 1lh;
  text-align: inherit;
}
::-webkit-datetime-edit {
  display: inline-flex;
}
::-webkit-datetime-edit-fields-wrapper {
  padding: 0;
}
::-webkit-datetime-edit, ::-webkit-datetime-edit-year-field, ::-webkit-datetime-edit-month-field, ::-webkit-datetime-edit-day-field, ::-webkit-datetime-edit-hour-field, ::-webkit-datetime-edit-minute-field, ::-webkit-datetime-edit-second-field, ::-webkit-datetime-edit-millisecond-field, ::-webkit-datetime-edit-meridiem-field {
  padding-block: 0;
}
::-webkit-calendar-picker-indicator {
  line-height: 1;
}
:-moz-ui-invalid {
  box-shadow: none;
}
button, input:where([type='button'], [type='reset'], [type='submit']), ::file-selector-button {
  appearance: button;
}
::-webkit-inner-spin-button, ::-webkit-outer-spin-button {
  height: auto;
}
[hidden]:where(:not([hidden='until-found'])) {
  display: none !important;
}
.pointer-events-auto {
  pointer-events: auto;
}
.pointer-events-none {
  pointer-events: none;
}
.absolute {
  position: absolute;
}
.fixed {
  position: fixed;
}
.inset-0 {
  inset: 0px;
}
.top-1\\/2 {
  top: calc(1 / 2 * 100%);
}
.top-2\\.5 {
  top: calc(var(--spacing) * 2.5);
}
.right-2 {
  right: calc(var(--spacing) * 2);
}
.left-1\\/2 {
  left: calc(1 / 2 * 100%);
}
.z-1 {
  z-index: 1;
}
.block {
  display: block;
}
.flex {
  display: flex;
}
.grid {
  display: grid;
}
.hidden {
  display: none;
}
.inline-flex {
  display: inline-flex;
}
.h-7 {
  height: calc(var(--spacing) * 7);
}
.h-8 {
  height: calc(var(--spacing) * 8);
}
.max-h-24 {
  max-height: calc(var(--spacing) * 24);
}
.max-h-\\[calc\\(100vh-16px\\)\\] {
  max-height: calc(100vh - 16px);
}
.max-h-\\[min\\(176px\\,calc\\(100vh-180px\\)\\)\\] {
  max-height: min(176px, calc(100vh - 180px));
}
.min-h-7 {
  min-height: calc(var(--spacing) * 7);
}
.min-h-8 {
  min-height: calc(var(--spacing) * 8);
}
.w-6 {
  width: calc(var(--spacing) * 6);
}
.w-8 {
  width: calc(var(--spacing) * 8);
}
.w-\\[min\\(360px\\,calc\\(100vw-16px\\)\\)\\] {
  width: min(360px, calc(100vw - 16px));
}
.w-full {
  width: 100%;
}
.max-w-70 {
  max-width: calc(var(--spacing) * 70);
}
.min-w-0 {
  min-width: 0px;
}
.flex-1 {
  flex: 1;
}
.shrink-0 {
  flex-shrink: 0;
}
.-translate-x-1\\/2 {
  --tw-translate-x: calc(calc(1 / 2 * 100%) * -1);
  translate: var(--tw-translate-x) var(--tw-translate-y);
}
.-translate-y-1\\/2 {
  --tw-translate-y: calc(calc(1 / 2 * 100%) * -1);
  translate: var(--tw-translate-x) var(--tw-translate-y);
}
.cursor-grab {
  cursor: grab;
}
.cursor-pointer {
  cursor: pointer;
}
.resize {
  resize: both;
}
.resize-none {
  resize: none;
}
.appearance-none {
  appearance: none;
}
.grid-cols-\\[22px_minmax\\(0\\,1fr\\)\\] {
  grid-template-columns: 22px minmax(0,1fr);
}
.grid-cols-\\[82px_minmax\\(0\\,1fr\\)\\] {
  grid-template-columns: 82px minmax(0,1fr);
}
.flex-col {
  flex-direction: column;
}
.items-center {
  align-items: center;
}
.items-start {
  align-items: flex-start;
}
.justify-center {
  justify-content: center;
}
.gap-0\\.5 {
  gap: calc(var(--spacing) * 0.5);
}
.gap-1 {
  gap: var(--spacing);
}
.gap-2 {
  gap: calc(var(--spacing) * 2);
}
.overflow-auto {
  overflow: auto;
}
.overflow-hidden {
  overflow: hidden;
}
.overflow-y-hidden {
  overflow-y: hidden;
}
.rounded-lg {
  border-radius: var(--t3-radius);
}
.rounded-md {
  border-radius: calc(var(--t3-radius) - 2px);
}
.rounded-xl {
  border-radius: calc(var(--t3-radius) + 4px);
}
.border {
  border-style: var(--tw-border-style);
  border-width: 1px;
}
.border-0 {
  border-style: var(--tw-border-style);
  border-width: 0px;
}
.border-t {
  border-top-style: var(--tw-border-style);
  border-top-width: 1px;
}
.border-b {
  border-bottom-style: var(--tw-border-style);
  border-bottom-width: 1px;
}
.border-border {
  border-color: var(--t3-border);
}
.border-input {
  border-color: var(--t3-input);
}
.border-primary {
  border-color: var(--t3-primary);
}
.border-transparent {
  border-color: transparent;
}
.border-b-transparent {
  border-bottom-color: transparent;
}
.bg-background {
  background-color: var(--t3-background);
}
.bg-muted {
  background-color: var(--t3-muted);
}
.bg-muted\\/40 {
  background-color: var(--t3-muted);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-muted) 40%, transparent);
  }
}
.bg-popover\\/95 {
  background-color: var(--t3-popover);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-popover) 95%, transparent);
  }
}
.bg-popover\\/96 {
  background-color: var(--t3-popover);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-popover) 96%, transparent);
  }
}
.bg-primary {
  background-color: var(--t3-primary);
}
.bg-primary\\/10 {
  background-color: var(--t3-primary);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-primary) 10%, transparent);
  }
}
.bg-transparent {
  background-color: transparent;
}
.p-0 {
  padding: 0px;
}
.p-1 {
  padding: var(--spacing);
}
.p-2 {
  padding: calc(var(--spacing) * 2);
}
.px-0 {
  padding-inline: 0px;
}
.px-1 {
  padding-inline: var(--spacing);
}
.px-2 {
  padding-inline: calc(var(--spacing) * 2);
}
.px-2\\.5 {
  padding-inline: calc(var(--spacing) * 2.5);
}
.px-3 {
  padding-inline: calc(var(--spacing) * 3);
}
.py-1 {
  padding-block: var(--spacing);
}
.py-1\\.5 {
  padding-block: calc(var(--spacing) * 1.5);
}
.py-2 {
  padding-block: calc(var(--spacing) * 2);
}
.font-mono {
  font-family: var(--t3-font-mono);
}
.font-sans {
  font-family: var(--t3-font-sans);
}
.text-lg {
  font-size: var(--text-lg);
  line-height: var(--tw-leading, var(--text-lg--line-height));
}
.text-sm {
  font-size: var(--text-sm);
  line-height: var(--tw-leading, var(--text-sm--line-height));
}
.text-xs {
  font-size: var(--text-xs);
  line-height: var(--tw-leading, var(--text-xs--line-height));
}
.leading-5 {
  --tw-leading: calc(var(--spacing) * 5);
  line-height: calc(var(--spacing) * 5);
}
.font-bold {
  --tw-font-weight: var(--font-weight-bold);
  font-weight: var(--font-weight-bold);
}
.font-medium {
  --tw-font-weight: var(--font-weight-medium);
  font-weight: var(--font-weight-medium);
}
.font-semibold {
  --tw-font-weight: var(--font-weight-semibold);
  font-weight: var(--font-weight-semibold);
}
.text-foreground {
  color: var(--t3-foreground);
}
.text-muted-foreground {
  color: var(--t3-muted-foreground);
}
.text-popover-foreground {
  color: var(--t3-popover-foreground);
}
.text-primary {
  color: var(--t3-primary);
}
.text-primary-foreground {
  color: var(--t3-primary-foreground);
}
.shadow-2xl {
  --tw-shadow: 0 25px 50px -12px var(--tw-shadow-color, rgb(0 0 0 / 0.25));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-lg {
  --tw-shadow: 0 10px 15px -3px var(--tw-shadow-color, rgb(0 0 0 / 0.1)), 0 4px 6px -4px var(--tw-shadow-color, rgb(0 0 0 / 0.1));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-md {
  --tw-shadow: 0 4px 6px -1px var(--tw-shadow-color, rgb(0 0 0 / 0.1)), 0 2px 4px -2px var(--tw-shadow-color, rgb(0 0 0 / 0.1));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-sm {
  --tw-shadow: 0 1px 3px 0 var(--tw-shadow-color, rgb(0 0 0 / 0.1)), 0 1px 2px -1px var(--tw-shadow-color, rgb(0 0 0 / 0.1));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-xs {
  --tw-shadow: 0 1px 2px 0 var(--tw-shadow-color, rgb(0 0 0 / 0.05));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.ring-0 {
  --tw-ring-shadow: var(--tw-ring-inset,) 0 0 0 calc(0px + var(--tw-ring-offset-width)) var(--tw-ring-color, currentcolor);
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.blur {
  --tw-blur: blur(8px);
  filter: var(--tw-blur,) var(--tw-brightness,) var(--tw-contrast,) var(--tw-grayscale,) var(--tw-hue-rotate,) var(--tw-invert,) var(--tw-saturate,) var(--tw-sepia,) var(--tw-drop-shadow,);
}
.backdrop-blur-xl {
  --tw-backdrop-blur: blur(var(--blur-xl));
  -webkit-backdrop-filter: var(--tw-backdrop-blur,) var(--tw-backdrop-brightness,) var(--tw-backdrop-contrast,) var(--tw-backdrop-grayscale,) var(--tw-backdrop-hue-rotate,) var(--tw-backdrop-invert,) var(--tw-backdrop-opacity,) var(--tw-backdrop-saturate,) var(--tw-backdrop-sepia,);
  backdrop-filter: var(--tw-backdrop-blur,) var(--tw-backdrop-brightness,) var(--tw-backdrop-contrast,) var(--tw-backdrop-grayscale,) var(--tw-backdrop-hue-rotate,) var(--tw-backdrop-invert,) var(--tw-backdrop-opacity,) var(--tw-backdrop-saturate,) var(--tw-backdrop-sepia,);
}
.outline-none {
  --tw-outline-style: none;
  outline-style: none;
}
.select-none {
  -webkit-user-select: none;
  user-select: none;
}
.placeholder\\:text-muted-foreground::placeholder {
  color: var(--t3-muted-foreground);
}
@media (hover: hover) {
  .hover\\:bg-accent:hover {
    background-color: var(--t3-accent);
  }
  .hover\\:bg-primary\\/90:hover {
    background-color: var(--t3-primary);
  }
  @supports (color: color-mix(in lab, red, red)) {
    .hover\\:bg-primary\\/90:hover {
      background-color: color-mix(in oklab, var(--t3-primary) 90%, transparent);
    }
  }
  .hover\\:text-accent-foreground:hover {
    color: var(--t3-accent-foreground);
  }
}
.focus\\:border-b-primary:focus {
  border-bottom-color: var(--t3-primary);
}
.focus\\:ring-0:focus {
  --tw-ring-shadow: var(--tw-ring-inset,) 0 0 0 calc(0px + var(--tw-ring-offset-width)) var(--tw-ring-color, currentcolor);
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.focus\\:outline-none:focus {
  --tw-outline-style: none;
  outline-style: none;
}
.disabled\\:pointer-events-none:disabled {
  pointer-events: none;
}
.disabled\\:opacity-60:disabled {
  opacity: 60%;
}
:host {
  --t3-font-sans: "DM Sans Variable", "DM Sans", -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui,
    sans-serif;
  --t3-font-mono: "SF Mono", "SFMono-Regular", "JetBrains Mono", Consolas, "Liberation Mono", Menlo, monospace;
  --t3-radius: 0.625rem;
  --t3-background: white;
  --t3-foreground: oklch(0.269 0 0);
  --t3-popover: white;
  --t3-popover-foreground: oklch(0.269 0 0);
  --t3-primary: oklch(0.488 0.217 264);
  --t3-primary-foreground: white;
  --t3-muted: rgb(0 0 0 / 4%);
  --t3-muted-foreground: oklch(0.556 0 0);
  --t3-accent: rgb(0 0 0 / 4%);
  --t3-accent-foreground: oklch(0.269 0 0);
  --t3-border: rgb(0 0 0 / 8%);
  --t3-input: rgb(0 0 0 / 10%);
  --t3-ring: oklch(0.488 0.217 264);
  color: var(--t3-foreground);
  font-family: var(--t3-font-sans);
}
* {
  box-sizing: border-box;
  border-color: var(--t3-border);
}
button, input, select, textarea {
  font: inherit;
}
button:focus-visible, input:focus-visible, select:focus-visible, textarea:focus-visible {
  outline: 2px solid var(--t3-ring);
  @supports (color: color-mix(in lab, red, red)) {
    outline: 2px solid color-mix(in srgb, var(--t3-ring) 72%, transparent);
  }
  outline-offset: 1px;
}
@property --tw-translate-x {
  syntax: "*";
  inherits: false;
  initial-value: 0;
}
@property --tw-translate-y {
  syntax: "*";
  inherits: false;
  initial-value: 0;
}
@property --tw-translate-z {
  syntax: "*";
  inherits: false;
  initial-value: 0;
}
@property --tw-border-style {
  syntax: "*";
  inherits: false;
  initial-value: solid;
}
@property --tw-leading {
  syntax: "*";
  inherits: false;
}
@property --tw-font-weight {
  syntax: "*";
  inherits: false;
}
@property --tw-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-shadow-color {
  syntax: "*";
  inherits: false;
}
@property --tw-shadow-alpha {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 100%;
}
@property --tw-inset-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-inset-shadow-color {
  syntax: "*";
  inherits: false;
}
@property --tw-inset-shadow-alpha {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 100%;
}
@property --tw-ring-color {
  syntax: "*";
  inherits: false;
}
@property --tw-ring-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-inset-ring-color {
  syntax: "*";
  inherits: false;
}
@property --tw-inset-ring-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-ring-inset {
  syntax: "*";
  inherits: false;
}
@property --tw-ring-offset-width {
  syntax: "<length>";
  inherits: false;
  initial-value: 0px;
}
@property --tw-ring-offset-color {
  syntax: "*";
  inherits: false;
  initial-value: #fff;
}
@property --tw-ring-offset-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-blur {
  syntax: "*";
  inherits: false;
}
@property --tw-brightness {
  syntax: "*";
  inherits: false;
}
@property --tw-contrast {
  syntax: "*";
  inherits: false;
}
@property --tw-grayscale {
  syntax: "*";
  inherits: false;
}
@property --tw-hue-rotate {
  syntax: "*";
  inherits: false;
}
@property --tw-invert {
  syntax: "*";
  inherits: false;
}
@property --tw-opacity {
  syntax: "*";
  inherits: false;
}
@property --tw-saturate {
  syntax: "*";
  inherits: false;
}
@property --tw-sepia {
  syntax: "*";
  inherits: false;
}
@property --tw-drop-shadow {
  syntax: "*";
  inherits: false;
}
@property --tw-drop-shadow-color {
  syntax: "*";
  inherits: false;
}
@property --tw-drop-shadow-alpha {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 100%;
}
@property --tw-drop-shadow-size {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-blur {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-brightness {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-contrast {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-grayscale {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-hue-rotate {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-invert {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-opacity {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-saturate {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-sepia {
  syntax: "*";
  inherits: false;
}
@layer properties {
  @supports ((-webkit-hyphens: none) and (not (margin-trim: inline))) or ((-moz-orient: inline) and (not (color:rgb(from red r g b)))) {
    *, ::before, ::after, ::backdrop {
      --tw-translate-x: 0;
      --tw-translate-y: 0;
      --tw-translate-z: 0;
      --tw-border-style: solid;
      --tw-leading: initial;
      --tw-font-weight: initial;
      --tw-shadow: 0 0 #0000;
      --tw-shadow-color: initial;
      --tw-shadow-alpha: 100%;
      --tw-inset-shadow: 0 0 #0000;
      --tw-inset-shadow-color: initial;
      --tw-inset-shadow-alpha: 100%;
      --tw-ring-color: initial;
      --tw-ring-shadow: 0 0 #0000;
      --tw-inset-ring-color: initial;
      --tw-inset-ring-shadow: 0 0 #0000;
      --tw-ring-inset: initial;
      --tw-ring-offset-width: 0px;
      --tw-ring-offset-color: #fff;
      --tw-ring-offset-shadow: 0 0 #0000;
      --tw-blur: initial;
      --tw-brightness: initial;
      --tw-contrast: initial;
      --tw-grayscale: initial;
      --tw-hue-rotate: initial;
      --tw-invert: initial;
      --tw-opacity: initial;
      --tw-saturate: initial;
      --tw-sepia: initial;
      --tw-drop-shadow: initial;
      --tw-drop-shadow-color: initial;
      --tw-drop-shadow-alpha: 100%;
      --tw-drop-shadow-size: initial;
      --tw-backdrop-blur: initial;
      --tw-backdrop-brightness: initial;
      --tw-backdrop-contrast: initial;
      --tw-backdrop-grayscale: initial;
      --tw-backdrop-hue-rotate: initial;
      --tw-backdrop-invert: initial;
      --tw-backdrop-opacity: initial;
      --tw-backdrop-saturate: initial;
      --tw-backdrop-sepia: initial;
    }
  }
}
`;var ii=(e)=>{if(e.key!=="Enter"||e.shiftKey||e.isComposing)return null;return e.metaKey||e.ctrlKey?"send":"attach"},M="data-zeron-annotation-ui",Re="data-zeron-annotation-tool",ai=2147483646,ve="var(--t3-primary)",mt="color-mix(in srgb, var(--t3-primary) 10%, transparent)",li=20,si=5000,Yn=4000,yt=1,Kn=10,De=null,Zn=0,gt=null,ui=(e,t)=>{if(!t)return;e.style.colorScheme=t.colorScheme;let n={"--t3-radius":t.radius,"--t3-background":t.background,"--t3-foreground":t.foreground,"--t3-popover":t.popover,"--t3-popover-foreground":t.popoverForeground,"--t3-primary":t.primary,"--t3-primary-foreground":t.primaryForeground,"--t3-muted":t.muted,"--t3-muted-foreground":t.mutedForeground,"--t3-accent":t.accent,"--t3-accent-foreground":t.accentForeground,"--t3-border":t.border,"--t3-input":t.input,"--t3-ring":t.ring,"--t3-font-sans":t.fontSans,"--t3-font-mono":t.fontMono};for(let[r,o]of Object.entries(n))e.style.setProperty(r,o)},$e=(e)=>(Zn+=1,`${e}_${Zn.toString(36)}`),He=(e)=>({x:e.left,y:e.top,width:e.width,height:e.height}),Jn=(e,t,n,r)=>({x:Math.min(e,n),y:Math.min(t,r),width:Math.abs(n-e),height:Math.abs(r-t)}),ci=(e)=>e.width>=3&&e.height>=3;function Qn(e,t=20){if(e.length===0)return null;let n=Math.min(...e.map((d)=>d.x)),r=Math.min(...e.map((d)=>d.y)),o=Math.max(...e.map((d)=>d.x+d.width)),i=Math.max(...e.map((d)=>d.y+d.height)),s=Math.max(0,n-t),u=Math.max(0,r-t),g=Math.max(1,window.innerWidth-s),b=Math.max(1,window.innerHeight-u);return{x:s,y:u,width:Math.min(g,o-n+t*2),height:Math.min(b,i-r+t*2)}}function oe(e){return e instanceof Element&&e.closest(`[${M}]`)!==null}function er(e,t){for(let n of document.elementsFromPoint(e,t)){if(!(n instanceof Element))continue;if(oe(n))continue;if(n===document.documentElement||n===document.body)continue;return n}return null}function di(e){let t=e.tagName.toLowerCase(),n=e.id?`#${e.id}`:"",r=e instanceof HTMLElement&&typeof e.className==="string"?e.className.trim().split(/\s+/).filter(Boolean).slice(0,2).map((o)=>`.${o}`).join(""):"";return`${t}${n}${r}`}function Oe(e,t){let n=document.createElement("div");return n.setAttribute(M,""),n.style.cssText=["position:fixed","pointer-events:none",`border:2px solid ${e}`,`background:${t}`,"border-radius:3px","box-sizing:border-box","display:none",`z-index:${yt}`].join(";"),n}function Ie(e,t){e.style.display="block",e.style.transform=`translate(${t.x}px, ${t.y}px)`,e.style.width=`${t.width}px`,e.style.height=`${t.height}px`}function pi(){let e=document.createElement("div");return e.setAttribute(M,""),e.className="fixed z-1 max-w-70 overflow-hidden rounded-md bg-primary px-2 py-1 font-sans text-xs font-semibold text-primary-foreground shadow-md",e.style.cssText=["position:fixed","pointer-events:none","white-space:nowrap","text-overflow:ellipsis",`z-index:${yt}`].join(";"),e}function ht(e){if(!e.element.isConnected){e.outline.style.display="none",e.label.style.display="none";return}let t=e.element.getBoundingClientRect();Ie(e.outline,He(t)),e.label.textContent=di(e.element),e.label.style.display="block",e.label.style.transform=`translate(${Math.max(4,t.left)}px, ${Math.max(4,t.top-22)}px)`}function fi(e){return{functionName:e.functionName??null,fileName:e.fileName??null,lineNumber:e.lineNumber??null,columnNumber:e.columnNumber??null}}function mi(e,t){let n;return Promise.race([e,new Promise((r)=>{n=setTimeout(()=>r(null),t)})]).finally(()=>clearTimeout(n))}var hi=500;async function gi(e){let t={pageUrl:location.href,pageTitle:document.title?.trim()||null,tagName:e.tagName.toLowerCase(),pickedAt:new Date().toISOString()};try{let n=await mi(Promise.resolve(Gn(e)),si);if(n){let r=(n.stack??[]).map(fi);return{...t,selector:n.selector,htmlPreview:(n.htmlPreview??"").slice(0,Yn),componentName:n.componentName,source:r.find((o)=>o.fileName)??null,stack:r,styles:(n.styles??"").slice(0,Yn)}}}catch{}return{...t,selector:null,htmlPreview:e.outerHTML.slice(0,hi),componentName:null,source:null,stack:[],styles:""}}function Pe(e,t){let n=document.createElement("button");return n.type="button",n.textContent=e,n.title=t,n.className="inline-flex h-7 cursor-pointer items-center justify-center rounded-md border border-transparent px-2 font-sans text-xs font-medium text-foreground outline-none hover:bg-accent disabled:pointer-events-none disabled:opacity-60",n}function bt(e){e.setAttribute("aria-label",e.getAttribute("aria-label")??"Style value"),e.className="h-7 min-w-0 w-full appearance-none rounded-md border border-input bg-background px-2 font-mono text-xs text-foreground shadow-xs outline-none"}function wt(e){let t=document.createElement("div");t.style.cssText="position:relative;min-width:0";let n=document.createElement("span");return n.textContent=e.dataset.unit??"",n.className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 font-mono text-xs text-muted-foreground",t.append(e,n),t}function V(e,t){let n=document.createElement("label");n.className="grid min-h-7 grid-cols-[82px_minmax(0,1fr)] items-center gap-2 font-sans text-xs font-medium text-muted-foreground";let r=document.createElement("span");return r.textContent=e,bt(t),n.append(r,t instanceof HTMLInputElement&&t.dataset.unit?wt(t):t),n}function ze(){let e=document.createElement("section");return e.className="grid gap-1 border-t border-border py-2",e}function xe(e,t="0"){let n=document.createElement("input");return n.type="number",n.placeholder=t,n.style.paddingRight="30px",n.dataset.unit=e,n}function bi(e){if(e.length===0)return"";if(e.length===1)return`M ${e[0].x} ${e[0].y} l 0.01 0.01`;let t=`M ${e[0].x} ${e[0].y}`;for(let r=1;r<e.length-1;r+=1){let o=e[r],i=e[r+1];t+=` Q ${o.x} ${o.y} ${(o.x+i.x)/2} ${(o.y+i.y)/2}`}let n=e[e.length-1];return t+=` L ${n.x} ${n.y}`,t}function wi(e,t){let n=e.map((b)=>b.x),r=e.map((b)=>b.y),o=t+3,i=Math.min(...n)-o,s=Math.min(...r)-o,u=Math.max(...n)+o,g=Math.max(...r)+o;return{x:i,y:s,width:u-i,height:g-s}}function yi(e){De?.teardown();let t=!1,n=!1,r=(a)=>{if(n)return;n=!0,e(JSON.stringify(a))},o=document.createElement("div");o.setAttribute(M,""),o.style.cssText=`position:fixed;inset:0;z-index:${ai};pointer-events:none`,ui(o,gt);let i=o.attachShadow({mode:"open"}),s=document.createElement("style");s.textContent=Xn,i.appendChild(s);let u=document.createElement("div");u.setAttribute(M,""),u.className="fixed inset-0 font-sans text-foreground",u.style.cssText="pointer-events:none";let g=document.createElement("style");g.setAttribute(M,""),g.textContent=`html[${Re}] body, html[${Re}] body * { cursor: crosshair !important; } [${M}], [${M}] * { cursor: default !important; } [${M}] input[type=number]::-webkit-inner-spin-button, [${M}] input[type=number]::-webkit-outer-spin-button { appearance:none; margin:0; }`,document.documentElement.appendChild(g),i.appendChild(u);let b=Oe(ve,mt),d=Oe(ve,mt);u.append(b,d);let f=document.createElementNS("http://www.w3.org/2000/svg","svg");f.setAttribute(M,""),f.setAttribute("width","100%"),f.setAttribute("height","100%"),f.setAttribute("viewBox",`0 0 ${window.innerWidth} ${window.innerHeight}`),f.style.cssText="position:fixed;inset:0;overflow:visible;pointer-events:none",f.style.zIndex=String(yt),u.appendChild(f);let p=document.createElement("div");p.setAttribute(M,""),p.className="pointer-events-auto fixed top-2.5 left-1/2 flex -translate-x-1/2 gap-0.5 rounded-lg border border-border bg-popover/95 p-1 text-popover-foreground shadow-lg backdrop-blur-xl",p.style.zIndex=String(Kn),u.appendChild(p);let h=document.createElement("div");h.setAttribute(M,""),h.className="pointer-events-auto fixed hidden max-h-[calc(100vh-16px)] w-[min(360px,calc(100vw-16px))] flex-col overflow-hidden rounded-xl border border-border bg-popover/96 text-popover-foreground shadow-2xl backdrop-blur-xl",h.style.zIndex=String(Kn),u.appendChild(h);let O=document.createElement("div");O.className="flex items-start gap-2 p-2";let A=Pe("","Expand annotation editor");A.setAttribute("aria-label","Expand annotation editor"),A.setAttribute("aria-expanded","false"),A.className+=" h-8 w-8 shrink-0 bg-muted p-0 text-muted-foreground hover:bg-accent hover:text-accent-foreground",A.innerHTML='<svg viewBox="0 0 20 20" width="15" height="15" aria-hidden="true"><path d="M4 5h12M4 10h12M4 15h12M7 3v4M13 8v4M9 13v4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>',O.appendChild(A);let _=document.createElement("textarea");_.placeholder="Describe the change…",_.rows=1,_.className="min-h-8 max-h-24 min-w-0 flex-1 resize-none overflow-y-hidden border-0 border-b border-b-transparent bg-transparent px-0 py-1.5 font-sans text-sm leading-5 text-foreground outline-none ring-0 placeholder:text-muted-foreground focus:border-b-primary focus:outline-none focus:ring-0",O.appendChild(_);let k=document.createElement("button");k.type="button",k.textContent="⠿",k.title="Drag annotation editor",k.className="hidden h-8 w-6 shrink-0 cursor-grab select-none border-0 bg-transparent p-0 font-sans text-lg font-bold leading-5 text-muted-foreground",O.appendChild(k);let B=Pe("Attach","Attach annotation and screenshot (Enter)");B.className+=" h-8 shrink-0 border-primary bg-primary px-3 text-primary-foreground shadow-sm hover:bg-primary/90",O.appendChild(B),h.appendChild(O);let W=document.createElement("div");W.className="hidden max-h-[min(176px,calc(100vh-180px))] overflow-auto border-t border-border bg-muted/40 px-3",h.appendChild(W);let S=new Map,U=[],q=[],ie=new Map,vt=new Map,N="select",R=null,T=null,xt=!1,j=!1,Et=!1,ae=null,F=null,J=null,tr=()=>{_.style.height="auto";let l=Math.min(_.scrollHeight,96);_.style.height=`${l}px`,_.style.overflowY=_.scrollHeight>96?"auto":"hidden",Se()};_.addEventListener("input",tr);let Q=()=>{let a=S.size>0||U.length>0||q.length>0;if(h.style.display=a?"flex":"none",B.disabled=!a,B.style.opacity=a?"1":"0.45",A.disabled=!a,W.style.display=j&&S.size>0?"grid":"none",Se(),a&&!Et)Et=!0,window.setTimeout(()=>_.focus({preventScroll:!0}),0)},je=()=>{for(let[a,l]of vt){let c=a===N;l.classList.toggle("bg-primary/10",c),l.classList.toggle("text-primary",c),l.classList.toggle("text-foreground",!c)}if(N!=="select")b.style.display="none";if(N!=="marquee")d.style.display="none";document.documentElement.setAttribute(Re,N)},Fe=(a)=>{if(a.element instanceof HTMLElement||a.element instanceof SVGElement)for(let[l,c]of a.baselineStyles)if(c)a.element.style.setProperty(l,c);else a.element.style.removeProperty(l);S.delete(a.element),a.outline.remove(),a.label.remove();for(let[l,c]of ie)if(c.targetId===a.id)ie.delete(l);Q()},kt=(a)=>{if(S.has(a))return;let l={id:$e("element"),element:a,outline:Oe(ve,mt),label:pi(),baselineStyles:new Map};if(S.set(a,l),u.append(l.outline,l.label),ht(l),Q(),j)W.style.display="grid",Nt()},nr=(a,l)=>{let c=S.get(a);if(c){Fe(c);return}if(!l)for(let m of Array.from(S.values()))Fe(m);kt(a)},L=(a,l)=>{for(let c of S.values()){if(!(c.element instanceof HTMLElement||c.element instanceof SVGElement))continue;if(!c.baselineStyles.has(a))c.baselineStyles.set(a,c.element.style.getPropertyValue(a));let m=`${c.id}:${a}`,w=ie.get(m)?.previousValue??getComputedStyle(c.element).getPropertyValue(a).trim();c.element.style.setProperty(a,l,"important"),ie.set(m,{targetId:c.id,selector:null,property:a,previousValue:w,value:l}),ht(c)}},le=ze(),Ee=ze(),ke=ze(),Ve=ze();W.append(le,Ee,ke,Ve);let ee=document.createElement("select");for(let a of["inherit","system-ui","sans-serif","serif","monospace"]){let l=document.createElement("option");l.value=a,l.textContent=a,ee.appendChild(l)}ee.addEventListener("change",()=>L("font-family",ee.value)),le.appendChild(V("Font",ee));let G=xe("px","16");G.min="1",G.max="300",G.addEventListener("input",()=>{if(G.value)L("font-size",`${G.value}px`)}),le.appendChild(V("Font size",G));let se=document.createElement("select");for(let a of["300","400","500","600","700","800","900"]){let l=document.createElement("option");l.value=a,l.textContent=a,se.appendChild(l)}se.addEventListener("change",()=>L("font-weight",se.value)),le.appendChild(V("Font weight",se));let X=document.createElement("input");X.type="text",X.placeholder="normal / 1.4",X.addEventListener("change",()=>{if(X.value.trim())L("line-height",X.value.trim())}),le.appendChild(V("Line height",X));let Be=(a,l,c)=>{let m=document.createElement("label");m.className="grid min-h-7 grid-cols-[82px_minmax(0,1fr)] items-center gap-2 font-sans text-xs font-medium text-muted-foreground";let w=document.createElement("span");w.textContent=a;let y=document.createElement("div");y.className="grid h-7 grid-cols-[22px_minmax(0,1fr)] items-center gap-1 rounded-md border border-input bg-background px-1 shadow-xs";let v=document.createElement("input");v.type="color",v.setAttribute("aria-label",a),v.style.cssText="width:20px;height:20px;padding:0;border:0;border-radius:5px;overflow:hidden;background:transparent;cursor:pointer";let C=document.createElement("input");return C.type="text",C.setAttribute("aria-label",`${a} value`),C.className="min-w-0 w-full border-0 bg-transparent font-mono text-xs text-foreground outline-none",v.addEventListener("input",()=>{C.value=v.value,L(l,v.value)}),C.addEventListener("change",()=>{let H=C.value.trim();if(!H)return;if(L(l,H),/^#[0-9a-f]{6}$/i.test(H))v.value=H}),y.append(v,C),m.append(w,y),c.appendChild(m),{row:m,color:v,text:C}},rr=Be("Text color","color",Ee),or=Be("Background","background-color",Ee),z=document.createElement("input");z.type="range",z.min="0",z.max="1",z.step="0.05",z.value="1",z.style.accentColor=ve,z.addEventListener("input",()=>L("opacity",z.value)),Ee.appendChild(V("Opacity",z));let Y=xe("px","0");Y.min="0",Y.max="300",Y.addEventListener("input",()=>{if(Y.value)L("border-radius",`${Y.value}px`)}),ke.appendChild(V("Radius",Y));let ir=Be("Border color","border-color",ke),K=xe("px","0");K.min="0",K.max="100",K.addEventListener("input",()=>{if(K.value)L("border-style","solid"),L("border-width",`${K.value}px`)}),ke.appendChild(V("Border width",K));let We=document.createElement("div");We.style.cssText="display:grid;grid-template-columns:82px minmax(0,1fr);gap:8px;align-items:center";let Ue=document.createElement("div");Ue.className="grid gap-2 font-sans text-xs font-medium text-muted-foreground",Ue.innerHTML="<span>Width</span><span>Height</span>";let qe=document.createElement("div");qe.style.cssText="position:relative;display:grid;gap:3px;padding-left:22px";let te=xe("px","auto"),ne=xe("px","auto");bt(te),bt(ne);let P=Pe("","Lock aspect ratio");P.setAttribute("aria-pressed","true"),P.style.cssText+=";position:absolute;left:0;top:50%;transform:translateY(-50%);width:18px;height:38px;padding:0",P.className+=" bg-primary/10 text-primary",qe.append(wt(te),wt(ne),P),We.append(Ue,qe),Ve.appendChild(We);let D=!0,ue=1,St=()=>{P.innerHTML=D?'<svg viewBox="0 0 20 20" width="14" height="14" aria-hidden="true"><path d="M8 6.5 9.5 5A3.5 3.5 0 0 1 14.5 10l-1.5 1.5M12 13.5 10.5 15A3.5 3.5 0 0 1 5.5 10L7 8.5M7.5 12.5l5-5" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>':'<svg viewBox="0 0 20 20" width="14" height="14" aria-hidden="true"><path d="m6 6 8 8M8 6.5 9.5 5A3.5 3.5 0 0 1 14 9M12 13.5 10.5 15A3.5 3.5 0 0 1 6 11" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>',P.setAttribute("aria-pressed",String(D)),P.classList.toggle("bg-primary/10",D),P.classList.toggle("text-primary",D),P.classList.toggle("bg-muted",!D),P.classList.toggle("text-muted-foreground",!D)};P.addEventListener("click",()=>{D=!D,St()}),te.addEventListener("input",()=>{let a=Number(te.value);if(!Number.isFinite(a)||a<=0)return;if(L("width",`${a}px`),D&&ue>0){let l=Math.max(1,Math.round(a/ue));ne.value=String(l),L("height",`${l}px`)}}),ne.addEventListener("input",()=>{let a=Number(ne.value);if(!Number.isFinite(a)||a<=0)return;if(L("height",`${a}px`),D&&ue>0){let l=Math.max(1,Math.round(a*ue));te.value=String(l),L("width",`${l}px`)}}),St();let Ge=(a,l,c)=>{let m=document.createElement("input");return m.type="text",m.placeholder=c,m.addEventListener("change",()=>{if(m.value.trim())L(l,m.value.trim())}),Ve.appendChild(V(a,m)),m},ar=Ge("Padding","padding","0 0 0 0"),lr=Ge("Margin","margin","0 0 0 0"),sr=Ge("Gap","gap","0px"),Nt=()=>{let a=S.values().next().value;if(!a)return;let l=getComputedStyle(a.element),c=a.element.getBoundingClientRect();ue=c.height>0?c.width/c.height:1,te.value=String(Math.round(c.width)),ne.value=String(Math.round(c.height)),G.value=String(Math.round(Number.parseFloat(l.fontSize)||16)),se.value=l.fontWeight.match(/^[0-9]+$/)?l.fontWeight:"400",X.value=l.lineHeight,ee.value=Array.from(ee.options).some((m)=>m.value===l.fontFamily)?l.fontFamily:"inherit",rr.text.value=l.color,or.text.value=l.backgroundColor,ir.text.value=l.borderColor,z.value=l.opacity,Y.value=String(Math.round(Number.parseFloat(l.borderRadius)||0)),K.value=String(Math.round(Number.parseFloat(l.borderWidth)||0)),ar.value=l.padding,lr.value=l.margin,sr.value=l.gap==="normal"?"0px":l.gap},ur=[["select","Select","Select elements (V)"],["marquee","Region","Draw a region or marquee-select elements (R)"],["draw","Draw","Draw freehand (D)"],["erase","Erase","Remove an annotation target (E)"]];for(let[a,l,c]of ur){let m=Pe(l,c);m.className+=" h-8 px-2.5 text-sm",m.addEventListener("click",()=>{N=a,je()}),vt.set(a,m),p.appendChild(m)}let cr=(a,l)=>{let m=h.getBoundingClientRect();return{left:Math.min(Math.max(8,a),Math.max(8,window.innerWidth-m.width-8)),top:Math.min(Math.max(8,l),Math.max(8,window.innerHeight-m.height-8))}},Xe=(a)=>{let l=cr(a.left,a.top);if(h.style.left=`${l.left}px`,h.style.top=`${l.top}px`,h.style.right="auto",h.style.bottom="auto",j)ae=l},dr=()=>Qn([...Array.from(S.values(),(a)=>He(a.element.getBoundingClientRect())),...U.map((a)=>a.rect),...q.map((a)=>a.bounds)],0),pr=()=>{let a=dr();if(!a)return;let l=h.getBoundingClientRect(),c=8,m=[{left:a.x+a.width+c,top:a.y},{left:a.x-l.width-c,top:a.y},{left:a.x+a.width-l.width,top:a.y+a.height+c},{left:a.x+a.width-l.width,top:a.y-l.height-c}],w=(v)=>Math.max(0,-v.left)+Math.max(0,-v.top)+Math.max(0,v.left+l.width-window.innerWidth)+Math.max(0,v.top+l.height-window.innerHeight),y=m.reduce((v,C)=>w(C)<w(v)?C:v);Xe(y)};function Se(){if(J!==null)window.cancelAnimationFrame(J);J=window.requestAnimationFrame(()=>{if(J=null,h.style.display==="none")return;if(j&&ae)Xe(ae);else pr()})}A.addEventListener("click",()=>{if(S.size===0)return;if(!j){let a=h.getBoundingClientRect();if(j=!0,ae={left:a.left,top:a.top},W.style.display=S.size>0?"grid":"none",k.style.display="block",A.setAttribute("aria-expanded","true"),A.title="Collapse annotation editor",A.setAttribute("aria-label","Collapse annotation editor"),S.size>0)Nt()}else j=!1,ae=null,W.style.display="none",k.style.display="none",A.setAttribute("aria-expanded","false"),A.title="Expand annotation editor",A.setAttribute("aria-label","Expand annotation editor");Se()});let At=(a)=>{if(a.button!==0||!j)return;let l=h.getBoundingClientRect();F={pointerId:a.pointerId,offsetX:a.clientX-l.left,offsetY:a.clientY-l.top},k.setPointerCapture(a.pointerId),k.style.cursor="grabbing",a.preventDefault(),a.stopPropagation()},Ct=(a)=>{if(!F||F.pointerId!==a.pointerId)return;Xe({left:a.clientX-F.offsetX,top:a.clientY-F.offsetY}),a.preventDefault(),a.stopPropagation()},Ne=(a)=>{if(!F||F.pointerId!==a.pointerId)return;if(F=null,k.style.cursor="grab",k.hasPointerCapture(a.pointerId))k.releasePointerCapture(a.pointerId);a.preventDefault(),a.stopPropagation()};k.addEventListener("pointerdown",At),k.addEventListener("pointermove",Ct),k.addEventListener("pointerup",Ne),k.addEventListener("pointercancel",Ne);let Ae=()=>{for(let a of S.values())ht(a);Se()},fr=(a,l)=>{for(let w of Array.from(S.values()).toReversed()){let y=w.element.getBoundingClientRect();if(a>=y.left&&a<=y.right&&l>=y.top&&l<=y.bottom)return Fe(w),!0}let c=U.findIndex((w)=>a>=w.rect.x&&a<=w.rect.x+w.rect.width&&l>=w.rect.y&&l<=w.rect.y+w.rect.height);if(c>=0){let[w]=U.splice(c,1);return u.querySelector(`[data-region-id="${w?.id}"]`)?.remove(),Q(),!0}let m=q.findIndex((w)=>a>=w.bounds.x&&a<=w.bounds.x+w.bounds.width&&l>=w.bounds.y&&l<=w.bounds.y+w.bounds.height);if(m>=0){let[w]=q.splice(m,1);return f.querySelector(`[data-stroke-id="${w?.id}"]`)?.remove(),Q(),!0}return!1},mr=(a)=>{let l=Array.from(document.querySelectorAll("body *")).filter((c)=>!oe(c)).map((c)=>({element:c,rect:c.getBoundingClientRect()})).filter(({rect:c})=>{if(c.width<2||c.height<2)return!1;return!(c.right<a.x||c.left>a.x+a.width||c.bottom<a.y||c.top>a.y+a.height)}).filter(({element:c,rect:m})=>{let w=m.left+m.width/2,y=m.top+m.height/2;return w>=a.x&&w<=a.x+a.width&&y>=a.y&&y<=a.y+a.height&&(c.children.length===0||c instanceof HTMLButtonElement||c instanceof HTMLAnchorElement||c.getAttribute("role")==="button")}).sort((c,m)=>c.rect.width*c.rect.height-m.rect.width*m.rect.height).slice(0,li);for(let c of l)kt(c.element);return l.length},ce=()=>{b.style.display="none"},Lt=(a)=>{if(oe(a.target)){ce();return}if(N==="select"&&R===null){let l=er(a.clientX,a.clientY);if(l)Ie(b,He(l.getBoundingClientRect()));else ce();return}if(ce(),N==="marquee"&&R){Ie(d,Jn(R.x,R.y,a.clientX,a.clientY));return}if(N==="draw"&&T)T.target.points=[...T.target.points,{x:a.clientX,y:a.clientY}],T.target.bounds=wi(T.target.points,T.target.width),T.path.setAttribute("d",bi(T.target.points))},Tt=(a)=>{if(a.button!==0||oe(a.target))return;if(a.preventDefault(),a.stopPropagation(),N==="select"){let l=er(a.clientX,a.clientY);if(l)nr(l,a.shiftKey);return}if(N==="erase"){fr(a.clientX,a.clientY);return}if(R={x:a.clientX,y:a.clientY},N==="draw"){let l={id:$e("stroke"),color:gt?.primary??"#2563eb",width:4,points:[R],bounds:{x:R.x,y:R.y,width:1,height:1}},c=document.createElementNS("http://www.w3.org/2000/svg","path");c.setAttribute(M,""),c.setAttribute("data-stroke-id",l.id),c.setAttribute("fill","none"),c.setAttribute("stroke",l.color),c.setAttribute("stroke-width",String(l.width)),c.setAttribute("stroke-linecap","round"),c.setAttribute("stroke-linejoin","round"),f.appendChild(c),T={target:l,path:c}}},Mt=(a)=>{if(!R)return;if(a.preventDefault(),a.stopPropagation(),N==="marquee"){let l=Jn(R.x,R.y,a.clientX,a.clientY);if(d.style.display="none",ci(l)){if(mr(l)===0){let m={id:$e("region"),rect:l};U.push(m);let w=Oe(ve,"color-mix(in srgb, var(--t3-primary) 6%, transparent)");w.setAttribute("data-region-id",m.id),Ie(w,l),u.appendChild(w)}}}else if(N==="draw"&&T){if(T.target.points.length>1)q.push(T.target);else T.path.remove();T=null}R=null,Q()},_t=(a)=>{if(oe(a.target))return;a.preventDefault(),a.stopPropagation()},Rt=(a)=>{if(a.relatedTarget===null)ce()},$t=()=>{ce()},hr=()=>{for(let a of S.values()){if(!(a.element instanceof HTMLElement||a.element instanceof SVGElement))continue;for(let[l,c]of a.baselineStyles)if(c)a.element.style.setProperty(l,c);else a.element.style.removeProperty(l)}},Ye=()=>{if(t)return;if(t=!0,hr(),window.removeEventListener("pointermove",Lt,!0),window.removeEventListener("pointerdown",Tt,!0),window.removeEventListener("pointerup",Mt,!0),window.removeEventListener("pointerout",Rt,!0),window.removeEventListener("click",_t,!0),window.removeEventListener("blur",$t),window.removeEventListener("keydown",Ot,!0),window.removeEventListener("scroll",Ae,!0),window.removeEventListener("resize",Ae),k.removeEventListener("pointerdown",At),k.removeEventListener("pointermove",Ct),k.removeEventListener("pointerup",Ne),k.removeEventListener("pointercancel",Ne),J!==null)window.cancelAnimationFrame(J);document.documentElement.removeAttribute(Re),g.remove(),o.remove(),De=null,r(null)},Ot=(a)=>{if(oe(a.target)&&a.key!=="Escape")return;if(a.key==="Escape"){a.preventDefault(),a.stopPropagation(),Ye();return}if(a.key==="v")N="select";else if(a.key==="r")N="marquee";else if(a.key==="d")N="draw";else if(a.key==="e")N="erase";else return;je()},Pt=(a)=>{if(xt||S.size===0&&U.length===0&&q.length===0)return;xt=!0,B.disabled=!0,B.textContent="Capturing…";let l=_.value.trim(),c=[...U],m=[...q],w=Array.from(ie.values(),(y)=>({...y}));Promise.all(Array.from(S.values()).map(async(y)=>{let v=await gi(y.element);for(let C of w)if(C.targetId===y.id&&v.selector!==null)C.selector=v.selector;return{id:y.id,element:v,rect:He(y.element.getBoundingClientRect())}})).then((y)=>{if(t)return;let v={id:$e("annotation"),pageUrl:location.href,pageTitle:document.title?.trim()||null,comment:l,elements:y,regions:c,strokes:m,styleChanges:w,screenshot:null,createdAt:new Date().toISOString()};h.style.display="none",p.style.display="none",b.style.display="none";let C=Qn([...y.map((H)=>H.rect),...c.map((H)=>H.rect),...m.map((H)=>H.bounds)]);window.requestAnimationFrame(()=>r({annotation:v,cropRect:C,submission:a}))}).catch(()=>Ye())};B.addEventListener("click",()=>Pt("attach")),u.addEventListener("keydown",(a)=>{let l=a.target===_?ii(a):null;if(a.stopImmediatePropagation(),!l)return;a.preventDefault(),Pt(l)}),window.addEventListener("pointermove",Lt,{capture:!0,passive:!1}),window.addEventListener("pointerdown",Tt,{capture:!0,passive:!1}),window.addEventListener("pointerup",Mt,{capture:!0,passive:!1}),window.addEventListener("pointerout",Rt,{capture:!0,passive:!0}),window.addEventListener("click",_t,{capture:!0,passive:!1}),window.addEventListener("blur",$t),window.addEventListener("keydown",Ot,{capture:!0}),window.addEventListener("scroll",Ae,{capture:!0,passive:!0}),window.addEventListener("resize",Ae,{passive:!0}),document.documentElement.appendChild(o),je(),Q(),De={teardown:Ye}}globalThis.__zeronAnnotate={start(e){return gt=e,new Promise((t)=>yi(t))},teardown(){De?.teardown()}};})();
