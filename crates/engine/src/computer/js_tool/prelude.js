(() => {
  const g = globalThis;

  const textOf = (result) =>
    (result.content || [])
      .filter((c) => c.type === "text")
      .map((c) => c.text)
      .join("\n");

  const parse = (text) => {
    try {
      return JSON.parse(text);
    } catch (_) {
      return null;
    }
  };

  const errorMessage = (result, data) => {
    if (result.isError) {
      const text = textOf(result);
      const failure = parse(text);
      if (failure && typeof failure.message === "string") {
        return typeof failure.error === "string" ? `${failure.error}: ${failure.message}` : failure.message;
      }
      return text || "The computer call failed.";
    }
    if (!data || typeof data !== "object") return null;
    if (data.error && typeof data.error === "object") {
      return data.error.message || data.error.code || "The computer call failed.";
    }
    if (typeof data.error === "string") return data.error;
    return null;
  };

  const refusalMessage = (structured, text) => {
    if (!structured) return null;
    if (structured.status === "refused") {
      const refusal = structured.refusal || {};
      return `${refusal.code || "refused"}: ${refusal.message || text || "The browser call was refused."}`;
    }
    if (structured.effect === "refused") {
      return `${structured.code || "refused"}: ${structured.message || text || "The action was refused."}`;
    }
    return null;
  };

  const call = (tool, args) => {
    const result = JSON.parse(__zeron_call(tool, JSON.stringify(args || {})));
    const first = (result.content || []).find((c) => c.type === "text");
    const data = first ? parse(first.text) : null;
    const structured = result.structuredContent && typeof result.structuredContent === "object" ? result.structuredContent : null;
    const message = errorMessage(result, data) || refusalMessage(structured, textOf(result));
    if (message) throw new Error(message);
    const images = (result.content || []).filter((c) => c.type === "image");
    return { data: data || structured || {}, structured: structured || data || {}, text: first ? first.text : "", images, raw: result };
  };

  const write = (value) => {
    if (value === undefined) return;
    __zeron_emit_text(typeof value === "string" ? value : describe(value));
  };

  const describe = (value) => {
    if (value === undefined || value === null) return "";
    if (value instanceof ZeronImage) return "";
    if (value instanceof App || value instanceof Tab || value instanceof Browser) return "";
    if (typeof value === "string") return value;
    if (typeof value === "number" || typeof value === "boolean") return String(value);
    try {
      return JSON.stringify(value);
    } catch (_) {
      return String(value);
    }
  };

  class ZeronImage {
    constructor(mimeType, data) {
      this.mimeType = mimeType;
      this.data = data;
      this.length = Math.floor((data.length * 3) / 4);
    }
  }

  const emitImage = (image) => {
    if (image instanceof ZeronImage) {
      __zeron_emit_image(image.mimeType, image.data);
    } else if (image && typeof image === "object" && image.data && image.mimeType) {
      __zeron_emit_image(image.mimeType, String(image.data));
    } else if (typeof image === "string" && image.startsWith("data:")) {
      const comma = image.indexOf(",");
      const mime = image.slice(5, image.indexOf(";"));
      __zeron_emit_image(mime || "image/png", image.slice(comma + 1));
    } else {
      throw new Error("emitImage expects an image returned by getScreenshot() or a data URL.");
    }
  };

  const collapse = (text, max) => {
    const flat = String(text).replace(/\s+/g, " ").trim();
    return flat.length > max ? flat.slice(0, max - 1) + "…" : flat;
  };

  const action = (tool, app, detail) =>
    __zeron_action(
      JSON.stringify(
        detail === undefined
          ? { tool, app }
          : { tool, app, detail: collapse(detail, 48) },
      ),
    );

  const XDOTOOL = {
    return: "enter",
    kp_enter: "enter",
    backspace: "backspace",
    escape: "escape",
    tab: "tab",
    space: "space",
    up: "up",
    down: "down",
    left: "left",
    right: "right",
    page_up: "pageup",
    prior: "pageup",
    page_down: "pagedown",
    next: "pagedown",
    home: "home",
    end: "end",
    delete: "delete",
    super: "meta",
    cmd: "meta",
    command: "meta",
    meta: "meta",
    ctrl: "ctrl",
    control: "ctrl",
    alt: "alt",
    option: "alt",
    shift: "shift",
  };

  const normalizeKey = (key) =>
    String(key)
      .split("+")
      .map((part) => {
        const lower = part.trim().toLowerCase();
        return XDOTOOL[lower] || (part.trim().length === 1 ? part.trim() : lower);
      })
      .join("+");

  const DIRECTIONS = { u: "up", d: "down", l: "left", r: "right" };
  const BUTTONS = { l: "left", r: "right", m: "middle" };

  const visibleWindows = (windows) =>
    (windows || []).filter(
      (w) =>
        w.visible !== false &&
        w.minimized !== true &&
        w.bounds &&
        w.bounds.width > 40 &&
        w.bounds.height > 40,
    );

  const findApp = (target) => {
    const apps = call("computer_list_apps").data.apps || [];
    const wanted = String(target).toLowerCase();
    const info =
      apps.find((a) => (a.bundleId || "").toLowerCase() === wanted) ||
      apps.find((a) => (a.name || "").toLowerCase() === wanted) ||
      apps.find((a) => (a.launchPath || "").toLowerCase() === wanted) ||
      apps.find((a) => (a.launchPath || "").toLowerCase().endsWith("/" + wanted + ".app"));
    const twins = info && info.running && info.bundleId ? apps.filter((a) => a.running && a.bundleId === info.bundleId) : [];
    if (twins.length > 1) {
      const windows = visibleWindows(call("computer_list_windows", { app: info.name }).data.windows || []).filter((w) =>
        twins.some((t) => t.pid === w.pid),
      );
      throw new Error(
        `Several ${info.name} instances are running. Pick a window with cua.getApp({ windowId }): ${windows
          .map((w) => `"${w.id}" (pid ${w.pid}, ${JSON.stringify(w.title || "")})`)
          .join(", ")}.`,
      );
    }
    return info;
  };

  const windowsFor = (appName, pid) => {
    const windows = call("computer_list_windows", { app: appName }).data.windows || [];
    return visibleWindows(windows.filter((w) => pid === undefined || w.pid === pid)).sort(
      (a, b) => (a.stackingIndex || 0) - (b.stackingIndex || 0),
    );
  };

  const inside = (inner, outer) =>
    inner &&
    outer &&
    inner.x >= outer.x &&
    inner.y >= outer.y &&
    inner.x + inner.width <= outer.x + outer.width &&
    inner.y + inner.height <= outer.y + outer.height;

  class App {
    constructor(window, appName) {
      this.windowId = window.id;
      this.pid = window.pid;
      this.appName = appName;
      this.title = window.title || "";
      this.screenshotId = null;
    }

    dialogNote() {
      const windows = call("computer_list_windows", { app: this.appName }).data.windows || [];
      const own = windows.find((w) => w.id === this.windowId);
      if (!own) return "";
      const dialogs = visibleWindows(windows).filter(
        (w) => w.id !== this.windowId && w.pid === own.pid && (w.title || "").trim() && inside(w.bounds, own.bounds),
      );
      return dialogs
        .map(
          (w) =>
            `\nDialog open over this window: "${w.title}" (window ${w.id}). Its buttons and text are in this state; press them by element number, e.g. click(<OK index>). Keyboard input to this app waits until it closes. Close any dialog your actions opened before you finish.`,
        )
        .join("");
    }

    getAXState(options) {
      const opts = options || {};
      const args = { window_id: this.windowId };
      if (opts.disableDiffing) args.full = true;
      const { data } = call("computer_get_state", args);
      action("get_state", this.appName);
      const state = renderState(data, this) + this.dialogNote();
      if (opts.emit !== false) write(state);
      return state;
    }

    getScreenshot(options) {
      const shot = call("computer_screenshot", { window_id: this.windowId });
      if (shot.data && shot.data.screenshotId) this.screenshotId = shot.data.screenshotId;
      else if (shot.data && shot.data.screenshot && shot.data.screenshot.screenshotId)
        this.screenshotId = shot.data.screenshot.screenshotId;
      const first = shot.images[0];
      const unchanged = shot.data && shot.data.screenshotUnchanged;
      if (!first && !(unchanged && this.lastImage)) throw new Error("No screenshot was returned for " + this.appName + ".");
      const image = first ? new ZeronImage(first.mimeType, first.data) : this.lastImage;
      this.lastImage = image;
      if (!options || options.emit !== false) emitImage(image);
      return image;
    }

    getAXStateAndScreenshot(options) {
      const state = this.getAXState(options);
      const screenshot = this.getScreenshot(options);
      return { state, screenshot };
    }

    ensureFrame() {
      if (!this.screenshotId) this.getScreenshot({ emit: false });
    }

    target(target, extra) {
      const args = Object.assign({ window_id: this.windowId, include_screenshot: false }, extra || {});
      if (typeof target === "number") args.ref = target;
      else if (Array.isArray(target)) {
        this.ensureFrame();
        args.x = target[0];
        args.y = target[1];
        args.screenshot_id = this.screenshotId;
      } else throw new Error("Target must be an element index or an [x, y] point.");
      return args;
    }

    click(target, options) {
      const opts = options || {};
      const extra = {};
      if (opts.mouseButton !== undefined)
        extra.button = BUTTONS[opts.mouseButton] || opts.mouseButton;
      if (opts.clickCount !== undefined) extra.count = opts.clickCount;
      call("computer_click", this.target(target, extra));
      action("click", this.appName);
    }

    drag(from, to) {
      this.ensureFrame();
      const point = (p) => ({
        x: p[0],
        y: p[1],
        window_id: this.windowId,
        screenshot_id: this.screenshotId,
      });
      call("computer_drag", { from: point(from), to: point(to), window_id: this.windowId });
      action("drag", this.appName);
    }

    scroll(target, direction, distance) {
      const dir = DIRECTIONS[direction] || direction;
      this.ensureFrame();
      let pixels;
      if (distance && typeof distance === "object" && distance.pixels) pixels = distance.pixels;
      else pixels = (typeof distance === "number" ? distance : 1) * 600;
      const args = this.target(target);
      args.delta_x = dir === "left" ? -pixels : dir === "right" ? pixels : 0;
      args.delta_y = dir === "up" ? -pixels : dir === "down" ? pixels : 0;
      if (args.screenshot_id === undefined) args.screenshot_id = this.screenshotId;
      call("computer_scroll", args);
      action("scroll", this.appName, dir);
    }

    selectText(elementIndex, text, options) {
      const opts = options || {};
      const args = { window_id: this.windowId, ref: elementIndex, text };
      if (opts.prefix !== undefined) args.prefix = opts.prefix;
      if (opts.suffix !== undefined) args.suffix = opts.suffix;
      if (opts.selectionType !== undefined) args.selection = opts.selectionType;
      call("computer_select_text", args);
      action("select_text", this.appName, text);
    }

    setValue(elementIndex, value) {
      call("computer_set_value", {
        window_id: this.windowId,
        ref: elementIndex,
        value: String(value),
        include_screenshot: false,
      });
      action("set_value", this.appName, value);
    }

    performSecondaryAction(elementIndex, name) {
      call("computer_perform_action", {
        window_id: this.windowId,
        ref: elementIndex,
        action: String(name),
      });
      action("perform_action", this.appName, name);
    }

    paste(text, options) {
      const args = { window_id: this.windowId, text: String(text) };
      if (options && options.format) args.format = options.format;
      call("computer_paste", args);
      action("paste", this.appName, text);
    }

    pressKey(key) {
      call("computer_press_key", {
        window_id: this.windowId,
        key: normalizeKey(key),
        include_screenshot: false,
      });
      action("press_key", this.appName, key);
    }

    typeText(text) {
      const { data } = call("computer_type_text", {
        window_id: this.windowId,
        text: String(text),
        include_screenshot: false,
      });
      action("type_text", this.appName, text);
      if (data && data.warning) write("Warning: " + data.warning);
    }
  }

  const renderState = (data, app) => {
    if (typeof data.tree === "string" && data.tree.length) return data.tree;
    const lines = [`Window: "${app.title}", App: ${app.appName}.`];
    for (const e of data.elements || []) {
      const value = e.value !== undefined && e.value !== null ? `, Value: ${e.value}` : "";
      lines.push(`${e.ref} ${String(e.role || "").replace(/^AX/, "").toLowerCase()} ${e.label || ""}${value}`);
    }
    if (data.elementChanges) lines.push("Changes: " + JSON.stringify(data.elementChanges));
    return lines.join("\n");
  };

  const BROWSER_BUNDLES = {
    "com.google.Chrome": "Chrome",
    "com.microsoft.edgemac": "Edge",
    "org.chromium.Chromium": "Chromium",
    "net.imput.helium": "Helium",
  };

  const APP_ONLY_BROWSERS = {
    "com.brave.Browser": "Brave",
    "company.thebrowser.dia": "Dia",
    "company.thebrowser.Browser": "Arc",
    "com.vivaldi.Vivaldi": "Vivaldi",
    "com.operasoftware.Opera": "Opera",
  };

  const runningBrowsers = () => {
    const apps = (call("computer_list_apps").data.apps || []).filter((a) => a.running && BROWSER_BUNDLES[a.bundleId]);
    return apps.map((a) => {
      const twins = apps.filter((b) => b.bundleId === a.bundleId).length > 1;
      const name = BROWSER_BUNDLES[a.bundleId];
      return {
        id: twins ? `${a.bundleId}#${a.pid}` : a.bundleId,
        name: twins ? `${name} (pid ${a.pid})` : name,
        family: name,
        pid: a.pid,
        appName: a.name,
      };
    });
  };

  const appOnlyBrowsers = (apps) =>
    apps
      .filter((a, i) => a.running && APP_ONLY_BROWSERS[a.bundleId] && apps.findIndex((b) => b.running && b.bundleId === a.bundleId) === i)
      .map((a) => ({
        id: a.bundleId,
        name: APP_ONLY_BROWSERS[a.bundleId],
        pageTool: false,
        reason: `${APP_ONLY_BROWSERS[a.bundleId]} is Chromium-based, but Zeron can only turn on page access for Chrome, Edge, Chromium and Helium. Drive it as an app with cua.getApp({ windowId }), or open the page in Chrome or the isolated browser.`,
      }));

  class Browser {
    constructor(info) {
      this.browserId = info.id;
      this.name = info.name;
      this.pid = info.pid;
      this.appName = info.appName;
      this.binding = null;
    }

    documentation() {
      return this.browserId === "isolated"
        ? "Isolated browser without the user's cookies. Tabs support getAXState, getScreenshot, click(ref), typeText(ref, text), pressKey(ref, \"Return\"), goto(url) and close()."
        : `The user's signed-in ${this.name} profile. Attaching asks the user once. Tabs you open close when this turn ends unless you call tab.markDeliverable() (keep it for the user) or tab.markHandoff() (keep it through the next turn). Claim an existing tab with cua.getTab(tabId, { browser: "${this.browserId}" }).`;
    }

    attach() {
      if (this.binding) return this.binding;
      if (this.browserId === "isolated") {
        const prepared = call("computer_browser_prepare", {
          allow_launch: true,
          profile: { mode: "isolated_named", name: "zeron" },
        }).structured;
        this.pid = prepared.prepared_pid || this.pid;
        if (!this.pid) throw new Error("The isolated browser did not report its process id.");
        const state = call("computer_browser_state", { pid: this.pid }).structured;
        this.binding = { target_id: state.target_id, tabs: state.tabs || [] };
      } else {
        const windows = windowsFor(this.appName, this.pid);
        const window = windows.find((w) => !windows.some((o) => o !== w && inside(w.bounds, o.bounds))) || windows[0];
        if (!window) throw new Error(`${this.name} has no open window to attach to.`);
        const windowNumber = Number(String(window.id).split(":").pop());
        call("computer_browser_prepare", {
          pid: this.pid,
          window_id: windowNumber,
          strategy: { kind: "existing_profile" },
        });
        const state = call("computer_browser_state", { pid: this.pid, window_id: windowNumber }).structured;
        this.binding = { target_id: state.target_id, tabs: state.tabs || [], window_id: windowNumber };
      }
      if (!this.binding.target_id) throw new Error(`Could not bind ${this.name}.`);
      return this.binding;
    }

    tabs() {
      return this.attach().tabs;
    }
  }

  const withoutEchoedImages = (outline) => {
    const parse = (line) => {
      const match = /^(\s*)- (\w+) \S+(?: "(.*?)")?/.exec(line);
      return match ? { indent: match[1].length, role: match[2], name: match[3] || "" } : null;
    };
    const kept = [];
    let parent = null;
    for (const line of outline.split("\n")) {
      const node = parse(line);
      const echo =
        node &&
        parent &&
        node.role === "image" &&
        parent.role === "link" &&
        node.indent > parent.indent &&
        (!node.name || node.name === parent.name);
      if (!echo) {
        kept.push(line);
        parent = node;
      }
    }
    return kept.join("\n");
  };

  const renderTabState = (snapshot, fallback) => {
    if (!snapshot || typeof snapshot !== "object") return fallback;
    const page = snapshot.page || {};
    const lines = [`Tab: "${page.title || ""}", URL: ${page.url || snapshot.url || ""}.`];
    if (typeof snapshot.outline === "string" && snapshot.outline.trim()) lines.push(withoutEchoedImages(snapshot.outline.trimEnd()));
    else
      for (const r of snapshot.refs || [])
        lines.push(`${r.ref} ${r.role || r.node || ""} ${r.name || r.label || ""}`.trimEnd());
    if (typeof snapshot.text_digest === "string" && snapshot.text_digest.trim())
      lines.push("Text:", snapshot.text_digest.trimEnd());
    const more = snapshot.snapshot && snapshot.snapshot.continuation;
    if (more) lines.push(`More of the page is not shown (continuation ${JSON.stringify(more)}).`);
    return lines.join("\n");
  };

  const boundBrowsers = new Map();
  const cachedBrowser = (info) => {
    const key = `${info.id}:${info.pid || ""}`;
    if (!boundBrowsers.has(key)) boundBrowsers.set(key, new Browser(info));
    return boundBrowsers.get(key);
  };

  const browserById = (id) => {
    if (!id || id === "isolated" || id === "iab") return cachedBrowser({ id: "isolated", name: "Isolated browser" });
    const wanted = String(id).toLowerCase();
    const running = runningBrowsers();
    const exact = running.filter((b) => b.id.toLowerCase() === wanted);
    const named = running.filter(
      (b) =>
        b.id.split("#")[0].toLowerCase() === wanted ||
        b.family.toLowerCase() === wanted ||
        b.appName.toLowerCase() === wanted,
    );
    const matches = exact.length ? exact : named;
    if (matches.length > 1) {
      throw new Error(
        `Several ${matches[0].family} instances are running. Pick one by id: ${matches.map((b) => `"${b.id}"`).join(", ")}.`,
      );
    }
    const found = matches[0];
    if (!found) {
      const other = appOnlyBrowsers(call("computer_list_apps").data.apps || []).find(
        (b) => b.id.toLowerCase() === wanted || b.name.toLowerCase() === wanted,
      );
      if (other) throw new Error(other.reason);
      throw new Error(`Browser "${id}" is not running. Use cua.listBrowsers() to see what is available.`);
    }
    return cachedBrowser(found);
  };

  class Tab {
    constructor(browser, tabId, title, url) {
      this.browser = browser;
      this.id = tabId;
      this.targetId = browser.binding.target_id;
      this.title = title || "";
      this.url = url || "";
    }

    ids(extra) {
      return Object.assign({ target_id: this.targetId, tab_id: this.id }, extra || {});
    }

    // Trusted CDP input would raise the user's browser window, so the driver only
    // admits the DOM route for a signed-in profile in the background.
    input(extra) {
      return this.ids(Object.assign(this.browser.browserId === "isolated" ? {} : { input_route: "dom_event" }, extra));
    }

    label() {
      return this.browser.name || "Browser";
    }

    getAXState(options) {
      const result = call("computer_browser_state", this.ids({ snapshot_format: "semantic_v2" }));
      const state = renderTabState(result.structured, result.text);
      if (!options || options.emit !== false) write(state);
      return state;
    }

    getScreenshot(options) {
      const shot = call("computer_browser_state", this.ids({ include_screenshot: true, screenshot_only: true }));
      const first = shot.images[0];
      if (!first) throw new Error("No tab screenshot was returned.");
      const image = new ZeronImage(first.mimeType, first.data);
      if (!options || options.emit !== false) emitImage(image);
      return image;
    }

    getAXStateAndScreenshot(options) {
      return { state: this.getAXState(options), screenshot: this.getScreenshot(options) };
    }

    click(ref) {
      if (typeof ref !== "number" && typeof ref !== "string")
        throw new Error("Tab click takes an element ref from getAXState().");
      call("computer_browser_click", this.input({ ref }));
      action("browser_click", this.label());
    }

    typeText(ref, text) {
      if (ref === null || ref === undefined) throw new Error("Pass the field ref from getAXState() to typeText.");
      call("computer_browser_type", this.input({ ref, text: String(text) }));
      action("browser_type", this.label(), text);
    }

    paste(ref, text) {
      return this.typeText(ref, text);
    }

    pressKey(ref, key) {
      const name = normalizeKey(key);
      if (name !== "enter") throw new Error("Browser tabs support pressKey(ref, \"Return\") only.");
      call("computer_browser_press", this.input({ ref }));
      action("browser_press", this.label(), key);
    }

    goto(url) {
      call("computer_browser_navigate", this.ids({ url: String(url) }));
      this.url = String(url);
      action("browser_navigate", this.label(), url);
    }

    close() {
      call("computer_browser_navigate", this.ids({ close_tab: true }));
      const binding = this.browser.binding;
      if (binding) binding.tabs = binding.tabs.filter((t) => t.tab_id !== this.id);
      action("browser_close", this.label());
    }

    markDeliverable() {
      call("computer_browser_state", this.ids({ keep: "deliverable" }));
    }

    markHandoff() {
      call("computer_browser_state", this.ids({ keep: "handoff" }));
    }
  }

  const cua = {
    computer: {
      target: "mac",
      launch_app: (input) => {
        call("computer_launch_app", { app: String(input.app) });
      },
    },

    getState(options) {
      const apps = (call("computer_list_apps").data.apps || [])
        .filter((a) => a.running)
        .map((a) => ({ id: a.bundleId || a.name, displayName: a.name, isRunning: true }));
      const windows = visibleWindows(call("computer_list_windows").data.windows || []);
      for (const app of apps) {
        const own = windows.filter((w) => w.appName === app.displayName);
        if (own.length) app.windows = own.map((w) => ({ id: w.id, app: w.appName, title: w.title }));
      }
      const browsers = runningBrowsers().map((b) => ({ id: b.id, name: b.name, type: "existing_profile" }));
      const state = { apps, browsers };
      const others = appOnlyBrowsers(call("computer_list_apps").data.apps || []);
      if (others.length) state.appOnlyBrowsers = others;
      if (!options || options.emit !== false) write(state);
      return state;
    },

    listApps(options) {
      const apps = (call("computer_list_apps").data.apps || []).map((a) => ({
        id: a.bundleId || a.name,
        displayName: a.name,
        isRunning: !!a.running,
        lastUsedDate: a.lastUsed,
      }));
      if (!options || options.emit !== false) write(apps);
      return apps;
    },

    listWindows(options) {
      const windows = visibleWindows(call("computer_list_windows").data.windows || []).map((w) => ({
        id: w.id,
        app: w.appName,
        title: w.title,
      }));
      if (!options || options.emit !== false) write(windows);
      return windows;
    },

    getApp(target) {
      let window;
      let appName;
      if (target && typeof target === "object" && target.windowId !== undefined) {
        const all = call("computer_list_windows").data.windows || [];
        window = all.find((w) => String(w.id) === String(target.windowId));
        if (!window) throw new Error("No open window has id " + target.windowId + ".");
        appName = window.appName;
      } else {
        const info = findApp(target);
        appName = info ? info.name : String(target);
        let windows = info && info.running ? windowsFor(appName, info.pid) : [];
        if (!windows.length) {
          const launched = call("computer_launch_app", { app: info ? info.bundleId || info.name : String(target) }).data;
          if (launched.window && launched.window.id) windows = [launched.window];
          else windows = windowsFor(appName, launched.pid);
        }
        window = windows[0];
        if (!window) throw new Error(`"${appName}" has no usable window. Open one, then call cua.getApp again.`);
      }
      const app = new App(window, appName);
      app.getAXState({ disableDiffing: true });
      return app;
    },

    listBrowsers(options) {
      const browsers = runningBrowsers()
        .map((b) => ({ id: b.id, name: b.name, family: "chrome", type: "existing_profile" }))
        .concat([{ id: "isolated", name: "Isolated browser", type: "cdp" }]);
      if (!options || options.emit !== false) write(browsers);
      return browsers;
    },

    getBrowser(options) {
      const browser = browserById(options && options.id);
      write(browser.documentation());
      return browser;
    },

    listTabs(options) {
      const browser = browserById(options && options.browser);
      const tabs = browser.tabs().map((t) => ({ id: t.tab_id, browserId: browser.browserId, title: t.title, url: t.url }));
      if (!options || options.emit !== false) write(tabs);
      return tabs;
    },

    getTab(reference, options) {
      const browser = browserById(options && options.browser);
      const tabs = browser.tabs();
      const found =
        typeof reference === "string"
          ? tabs.find((t) => t.tab_id === reference)
          : reference && reference.url
            ? tabs.find((t) => t.url === reference.url) ||
              tabs.find((t) => String(t.url || "").startsWith(reference.url))
            : null;
      if (!found) throw new Error("No open tab matches that reference.");
      const tab = new Tab(browser, found.tab_id, found.title, found.url);
      tab.getAXState();
      return tab;
    },

    async createBrowserTab(browserId, url, options) {
      const browser = browserById(browserId);
      const binding = browser.attach();
      const anchor = binding.tabs.find((t) => t.active) || binding.tabs[0];
      if (!anchor) throw new Error(`${browser.name} has no tab to anchor a new tab on.`);
      let tab;
      if (browser.browserId === "isolated") {
        tab = new Tab(browser, anchor.tab_id, anchor.title, anchor.url);
        if (url) tab.goto(url);
      } else {
        const opened = call("computer_browser_navigate", {
          target_id: binding.target_id,
          tab_id: anchor.tab_id,
          url: String(url || "about:blank"),
          new_tab: true,
        }).structured;
        tab = new Tab(browser, opened.tab_id, "", url);
        binding.tabs.push({ tab_id: opened.tab_id, title: "", url: String(url || "about:blank") });
        action("browser_navigate", browser.name, url);
      }
      for (let attempt = 0; ; attempt++) {
        try {
          tab.getAXState();
          return tab;
        } catch (error) {
          if (attempt >= 10 || !/origin could not be proven/.test(error.message)) throw error;
          await new Promise((resolve) => setTimeout(resolve, 1000));
        }
      }
    },
  };

  const timers = [];
  let nextTimer = 1;
  const schedule = (fn, ms, args, every) => {
    if (typeof fn !== "function") throw new TypeError("Timer callback must be a function.");
    const id = nextTimer++;
    const delay = Math.max(0, Number(ms) || 0);
    timers.push({ id, at: Date.now() + delay, fn, args, every: every ? delay : null });
    return id;
  };
  const cancel = (id) => {
    const index = timers.findIndex((t) => t.id === id);
    if (index >= 0) timers.splice(index, 1);
  };
  g.setTimeout = (fn, ms, ...args) => schedule(fn, ms, args, false);
  g.setInterval = (fn, ms, ...args) => schedule(fn, ms, args, true);
  g.clearTimeout = cancel;
  g.clearInterval = cancel;
  g.__zeron_run_timers = () => {
    const now = Date.now();
    const due = timers.filter((t) => t.at <= now).sort((a, b) => a.at - b.at);
    for (const timer of due) {
      if (timer.every !== null) timer.at = now + Math.max(1, timer.every);
      else cancel(timer.id);
      timer.fn(...timer.args);
    }
    return due.length > 0;
  };

  g.cua = cua;
  g.nodeRepl = { write, emitImage };
  g.__zeron_describe = describe;
})();
