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
    if (result.isError) return textOf(result) || "The computer call failed.";
    if (!data || typeof data !== "object") return null;
    if (data.error && typeof data.error === "object") {
      return data.error.message || data.error.code || "The computer call failed.";
    }
    if (typeof data.error === "string") return data.error;
    return null;
  };

  const call = (tool, args) => {
    const result = JSON.parse(__zeron_call(tool, JSON.stringify(args || {})));
    const first = (result.content || []).find((c) => c.type === "text");
    const data = first ? parse(first.text) : null;
    const message = errorMessage(result, data);
    if (message) throw new Error(message);
    const images = (result.content || []).filter((c) => c.type === "image");
    return { data: data || {}, text: first ? first.text : "", images, raw: result };
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
    return (
      apps.find((a) => (a.bundleId || "").toLowerCase() === wanted) ||
      apps.find((a) => (a.name || "").toLowerCase() === wanted) ||
      apps.find((a) => (a.launchPath || "").toLowerCase() === wanted) ||
      apps.find((a) => (a.launchPath || "").toLowerCase().endsWith("/" + wanted + ".app"))
    );
  };

  const windowsFor = (appName, pid) => {
    const windows = call("computer_list_windows", { app: appName }).data.windows || [];
    return visibleWindows(windows.filter((w) => pid === undefined || w.pid === pid)).sort(
      (a, b) => (a.stackingIndex || 0) - (b.stackingIndex || 0),
    );
  };

  class App {
    constructor(window, appName) {
      this.windowId = window.id;
      this.appName = appName;
      this.title = window.title || "";
      this.screenshotId = null;
    }

    getAXState(options) {
      const opts = options || {};
      const args = { window_id: this.windowId };
      if (opts.disableDiffing) args.diff = false;
      const { data } = call("computer_get_state", args);
      const state = renderState(data, this);
      if (opts.emit !== false) write(state);
      return state;
    }

    getScreenshot(options) {
      const shot = call("computer_screenshot", { window_id: this.windowId });
      if (shot.data && shot.data.screenshotId) this.screenshotId = shot.data.screenshotId;
      else if (shot.data && shot.data.screenshot && shot.data.screenshot.screenshotId)
        this.screenshotId = shot.data.screenshot.screenshotId;
      const first = shot.images[0];
      if (!first) throw new Error("No screenshot was returned for " + this.appName + ".");
      const image = new ZeronImage(first.mimeType, first.data);
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
      call("computer_type_text", {
        window_id: this.windowId,
        text: String(text),
        include_screenshot: false,
      });
      action("type_text", this.appName, text);
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

  class Browser {
    constructor(id, name) {
      this.browserId = id;
      this.name = name;
    }

    documentation() {
      return "Browser tabs support getAXState, getScreenshot, click(ref), typeText(ref, text), pressKey(ref, key), goto(url) and close().";
    }
  }

  class Tab {
    constructor(binding, title, url) {
      this.id = binding.tab_id;
      this.targetId = binding.target_id;
      this.title = title;
      this.url = url;
    }

    ids(extra) {
      return Object.assign({ target_id: this.targetId, tab_id: this.id }, extra || {});
    }

    getAXState(options) {
      const { data, text } = call("computer_browser_state", this.ids());
      const state = data.snapshot || data.tree || text;
      if (!options || options.emit !== false) write(state);
      return state;
    }

    getScreenshot(options) {
      const shot = call("computer_browser_state", this.ids({ include_screenshot: true }));
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
      call("computer_browser_click", this.ids({ ref }));
      action("browser_click", this.title || "Browser");
    }

    typeText(ref, text) {
      if (ref === null) throw new Error("Pass the field ref from getAXState() to typeText.");
      call("computer_browser_type", this.ids({ ref, text: String(text) }));
      action("browser_type", this.title || "Browser", text);
    }

    paste(ref, text) {
      return this.typeText(ref, text);
    }

    pressKey(ref, key) {
      const name = normalizeKey(key);
      if (name !== "enter") throw new Error("Browser tabs support pressKey(ref, \"Return\") only.");
      call("computer_browser_press", this.ids({ ref }));
      action("browser_press", this.title || "Browser", key);
    }

    goto(url) {
      call("computer_browser_navigate", this.ids({ url: String(url) }));
      this.url = String(url);
      action("browser_navigate", this.title || "Browser", url);
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
      const state = { apps, browsers: [] };
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
      const browsers = [{ id: "isolated", name: "Isolated browser", type: "cdp" }];
      if (!options || options.emit !== false) write(browsers);
      return browsers;
    },

    getBrowser(options) {
      const id = (options && options.id) || "isolated";
      const browser = new Browser(id, "Isolated browser");
      write(browser.documentation());
      return browser;
    },

    createBrowserTab(browserId, url, options) {
      const prepared = call("computer_browser_prepare", {
        allow_launch: true,
        profile: { mode: "isolated_named", name: "zeron" },
      }).data;
      const binding = prepared.binding || prepared;
      const tab = new Tab(
        { target_id: binding.target_id, tab_id: binding.tab_id || (binding.tabs && binding.tabs[0] && binding.tabs[0].tab_id) },
        "",
        url,
      );
      if (url) tab.goto(url);
      tab.getAXState();
      return tab;
    },
  };

  g.cua = cua;
  g.nodeRepl = { write, emitImage };
  g.__zeron_describe = describe;
})();
