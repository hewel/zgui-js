import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { bundle } from "../../packages/build/src/build.ts";

const BUBBLE_EXEMPT = new Set(["pointerenter", "pointerleave"]);

function resolveEventSpec(name) {
  const raw = String(name ?? "");
  const stem = raw.replace(/^on/, "");
  const lower = stem.toLowerCase();
  if (lower === "focus" || lower === "focusin") {
    return {
      prop: "onFocus",
      captureProp: "onFocusCapture",
      type: lower === "focus" ? "focusin" : lower,
      bubbles: true,
    };
  }
  if (lower === "blur" || lower === "focusout") {
    return {
      prop: "onBlur",
      captureProp: "onBlurCapture",
      type: lower === "blur" ? "focusout" : lower,
      bubbles: true,
    };
  }
  const pascal = stem.charAt(0).toUpperCase() + stem.slice(1);
  return {
    prop: `on${pascal}`,
    captureProp: `on${pascal}Capture`,
    type: lower,
    bubbles: !BUBBLE_EXEMPT.has(lower),
  };
}

function eventFamilyDefaults(type, extras) {
  if (type === "keydown" || type === "keyup") {
    return {
      key: null,
      character: null,
      keyWithoutModifiers: extras.key ?? null,
      deadKey: null,
      code: null,
      physicalCode: null,
      location: "standard",
      repeat: false,
      isComposing: false,
    };
  }
  if (type.startsWith("pointer") || type === "contextmenu") {
    return {
      position: { x: 0, y: 0 },
      pointerId: "1",
      pointerKind: "mouse",
      primary: true,
      button: "primary",
      pressure: null,
      coalesced: [],
      activateNow() {},
      capturePointer() {},
      releasePointer() {},
    };
  }
  if (type === "wheel") {
    return {
      position: { x: 0, y: 0 },
      pointerId: "1",
      pointerKind: "mouse",
      delta: { x: 0, y: 0, unit: "lines" },
      scrollPhase: "discrete",
    };
  }
  if (type === "focusin" || type === "focusout") {
    return { cause: "programmatic" };
  }
  if (type.startsWith("animation") || type.startsWith("transition")) {
    return { animationName: "", propertyName: "", elapsedTime: 0, pseudoElement: "" };
  }
  if (type === "input" || type === "change") {
    return { value: "", selection: { start: 0, end: 0 }, valueChange: "input" };
  }
  return {};
}

export function createEvent(nameOrType, target, extras = {}) {
  const spec = resolveEventSpec(nameOrType);
  const type = extras.type ?? spec.type;
  let prevented = false;
  let stopped = false;
  let immediate = false;
  let phase = "target";
  let currentTarget = target;
  const modifiers = extras.modifiers ?? { shift: false, control: false, alt: false, meta: false };
  const snapshot = (current, dispatchPhase, isActive) => ({
    type,
    timestamp: extras.timestamp ?? 0,
    target,
    modifiers,
    ...eventFamilyDefaults(type, extras),
    ...extras,
    phase: dispatchPhase,
    currentTarget: current,
    get defaultPrevented() {
      return prevented;
    },
    get propagationStopped() {
      return stopped;
    },
    get immediatePropagationStopped() {
      return immediate;
    },
    preventDefault() {
      requireActive(isActive);
      prevented = true;
    },
    stopPropagation() {
      requireActive(isActive);
      stopped = true;
    },
    stopImmediatePropagation() {
      requireActive(isActive);
      immediate = true;
      stopped = true;
    },
  });
  const requireActive = isActive => {
    if (!isActive())
      throw new ReferenceError("expired-dispatch: native event cancellation is synchronous");
  };
  return {
    spec,
    get event() {
      return snapshot(currentTarget, phase, () => false);
    },
    invoke(handler, nextPhase, nextCurrent) {
      phase = nextPhase;
      currentTarget = nextCurrent;
      if (!handler) return;
      let active = true;
      try {
        handler(snapshot(currentTarget, phase, () => active));
      } finally {
        active = false;
      }
    },
    isStopped: () => stopped,
    isImmediate: () => immediate,
  };
}

function makeLease(collection, fields) {
  const record = { ...fields, active: true, disposals: 0 };
  let disposed = false;
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    record.disposals += 1;
    record.active = false;
  };
  record._dispose = dispose;
  collection.push(record);
  return { record, dispose };
}

function textOf(node, texts) {
  if (texts.has(node)) return texts.get(node);
  if (node.tag === "#text") return node.props.get("text") ?? "";
  return node.children.map(child => textOf(child, texts)).join("");
}

const nativeQueueMicrotask = globalThis.queueMicrotask.bind(globalThis);
const hostMicrotasks = new WeakMap();
globalThis.queueMicrotask = fn => {
  const queue = hostMicrotasks.get(globalThis.__zguiShell) ?? hostMicrotasks.get(globalThis.__zgui);
  if (queue) queue.push(fn);
  else nativeQueueMicrotask(fn);
};

let nextCanvasToken = 0;
let nextSessionGeneration = 1;
let nextLockRequestId = 1;

export function createHost({
  strictStyles = false,
  reducedMotion = false,
  shellReducedMotion = false,
  backgroundBlurSupported = false,
  monitors = [],
  sessionLock,
  shellSurfaces = true,
} = {}) {
  if (!Number.isSafeInteger(nextSessionGeneration))
    throw new RangeError("session generation exhausted");
  const generation = nextSessionGeneration++;
  let nextId = 0;
  let stopping = false;
  let turnDepth = 0;
  let inCleanup = false;
  let cleanupJobsPending = false;
  const microtasks = [];
  const drainMicrotasks = () => {
    while (microtasks.length) {
      const batch = microtasks.splice(0, microtasks.length);
      for (const task of batch) task();
    }
  };
  const requireShellWorkAllowed = op => {
    assertActiveHost(op);
    if (inCleanup || cleanupJobsPending)
      throw new TypeError("shell work is unavailable during disposal");
  };
  const nodes = new Map();
  const parents = new Map();
  const nodeRegions = new WeakMap();
  const stylesheets = new Map();
  const installs = [];
  const primary = { stylesheets, installs };
  const focused = { current: null, reveal: false };
  const rects = new Map();
  const texts = new Map();
  const animating = new Map();
  const invalidated = new Set();
  const pendingRemoval = new Set();
  const portals = [];
  const traps = [];
  const locks = [];
  const scrollCommands = [];
  const windowCommands = [];
  const regionWindows = new WeakMap();
  const windowStateFor = region => {
    if (!regionWindows.has(region)) {
      regionWindows.set(region, {
        state: {
          title: "zgui-js",
          fullscreen: false,
          maximized: false,
          minimized: false,
          decorations: "full",
        },
        stateSubscribers: new Set(),
        closeHandlers: new Set(),
        closed: false,
      });
    }
    return regionWindows.get(region);
  };
  const notifyWindowState = win => {
    const snapshot = { ...win.state };
    for (const cb of [...win.stateSubscribers]) cb(snapshot);
  };
  const runCloseHandlers = region => {
    const win = windowStateFor(region);
    let prevented = false;
    const event = {
      get defaultPrevented() {
        return prevented;
      },
      preventDefault() {
        prevented = true;
      },
    };
    for (const handler of [...win.closeHandlers]) handler(event);
    return prevented;
  };
  const selections = new Map();
  const editableTextOf = node => {
    const value = node.props.get("value");
    if (typeof value === "string") return value;
    const def = node.props.get("defaultValue");
    return typeof def === "string" ? def : "";
  };
  const assertEditable = (node, op) => {
    if (node.tag !== "field" && node.tag !== "editor") {
      throw new TypeError(`${op} requires a field or editor`);
    }
  };
  const clampUtf16Boundary = (text, offset) => {
    const clamped = Math.max(0, Math.min(text.length, Math.trunc(offset)));
    if (clamped > 0 && clamped < text.length) {
      const prev = text.charCodeAt(clamped - 1);
      const curr = text.charCodeAt(clamped);
      if (prev >= 0xd800 && prev <= 0xdbff && curr >= 0xdc00 && curr <= 0xdfff) {
        return clamped - 1;
      }
    }
    return clamped;
  };
  const layers = [];
  const geometry = [];
  const animatingQueries = [];
  const canvasScenes = [];
  const scenesByHandle = new WeakMap();
  const queries = { rect: [], text: [], precedes: [], animating: animatingQueries };
  const clearCalls = [];
  const registrationsAfterStop = [];
  const leaseGroups = [portals, traps, locks, layers, geometry];
  const styleDiagnostics = [];
  const styling = new WeakMap();
  const stateFor = node => {
    if (!styling.has(node))
      styling.set(node, {
        aliases: {},
        applied: null,
        internal: null,
        defaults: {},
        application: {},
        invariants: {},
        direct: {},
      });
    return styling.get(node);
  };
  const failStyle = (code, message) => {
    styleDiagnostics.push({ code, message });
    if (strictStyles) throw new TypeError(`${code}: ${message}`);
  };
  // This fixture records declarations; actual CSS grammar/atomic commit is tested on DocumentDom.
  const normalizeStyle = value => {
    if (value == null) return {};
    if (typeof value === "string") {
      const result = {};
      for (const declaration of value.split(";").filter(part => part.trim())) {
        const colon = declaration.indexOf(":");
        if (colon < 0) throw new TypeError("invalid declaration");
        result[declaration.slice(0, colon).trim()] = declaration
          .slice(colon + 1)
          .trim()
          .replace(/\s*!important$/, "");
      }
      value = result;
    }
    if (typeof value !== "object" || Array.isArray(value))
      throw new TypeError("style must be a string or object");
    const result = {};
    const unitless = new Set([
      "opacity",
      "flex-grow",
      "flex-shrink",
      "font-weight",
      "line-height",
      "z-index",
      "order",
      "animation-iteration-count",
      "scale",
    ]);
    for (const [key, raw] of Object.entries(value)) {
      const name = key.startsWith("--")
        ? key
        : key.replace(/[A-Z]/g, match => `-${match.toLowerCase()}`);
      if (raw == null) continue;
      if (
        typeof raw === "number" &&
        (!Number.isFinite(raw) || !(name.startsWith("--") || unitless.has(name)))
      )
        throw new TypeError(`invalid numeric ${name}`);
      if (typeof raw !== "string" && typeof raw !== "number")
        throw new TypeError(`invalid ${name}`);
      const text = String(raw);
      if (
        /\b(?:NaN|Infinity)(?:[a-z%]*)\b|\b(?:url|image-set)\s*\(/i.test(text) ||
        /^(?:invalid|not-a-colour)$/.test(text) ||
        name === "unknown-property" ||
        name === "colour"
      )
        throw new TypeError(`invalid native declaration ${name}`);
      result[name] = text;
    }
    return result;
  };
  const commitStyle = (node, state) => {
    const styles = {
      ...state.defaults,
      ...state.application,
      ...state.direct,
      ...state.invariants,
    };
    for (const key of node.props.keys()) if (key.startsWith("--")) node.props.delete(key);
    for (const [key, value] of Object.entries(styles))
      if (key.startsWith("--")) node.props.set(key, value);
    if (Object.keys(styles).length) node.props.set("style", styles);
    else node.props.delete("style");
  };

  const assertLive = (node, role = "node", region) => {
    if (!node || typeof node !== "object" || nodes.get(node.id) !== node) {
      throw new TypeError(`invalid native ${role}`);
    }
    if (invalidated.has(node)) {
      throw new TypeError(`stale detached native ${role}`);
    }
    if (region && nodeRegions.get(node) !== region) {
      throw new TypeError(`foreign mount region for native ${role}`);
    }
  };

  const assertActiveHost = op => {
    if (stopping) throw new ReferenceError(`native session is stopping during ${op}`);
  };

  const makeNode = (tag, region = primary) => {
    const node = { tag, id: ++nextId, props: new Map(), callbacks: {}, children: [] };
    nodes.set(node.id, node);
    nodeRegions.set(node, region);
    return node;
  };

  const collectSubtree = (root, out = new Set()) => {
    out.add(root);
    for (const child of root.children) collectSubtree(child, out);
    return out;
  };

  const invalidate = doomed => {
    for (const group of leaseGroups) {
      for (const record of group) {
        if (doomed.has(record.node)) record._dispose();
      }
    }
    for (const node of doomed) invalidated.add(node);
  };

  const endTurn = () => {
    if (pendingRemoval.size === 0) return;
    const roots = [...pendingRemoval];
    pendingRemoval.clear();
    const doomed = new Set();
    for (const candidate of roots) {
      if (!parents.has(candidate) && candidate.tag !== "root") collectSubtree(candidate, doomed);
    }
    invalidate(doomed);
  };

  const runTurn = (fn, checkpoint = true) => {
    turnDepth += 1;
    try {
      return fn();
    } finally {
      try {
        if (turnDepth === 1 && !stopping && checkpoint) {
          drainMicrotasks();
          cleanupJobsPending = false;
        }
      } finally {
        turnDepth -= 1;
        if (turnDepth === 0) endTurn();
      }
    }
  };

  const dispatch = (target, nameOrType, extras = {}) =>
    runTurn(() => {
      assertLive(target, "event target");
      const path = [];
      for (let current = target; current; current = parents.get(current)) path.push(current);
      const carrier = createEvent(nameOrType, target, extras);
      const { spec } = carrier;
      const bubbles = extras.bubbles ?? spec.bubbles;
      for (let i = path.length - 1; i >= 1 && !carrier.isStopped(); i -= 1) {
        carrier.invoke(path[i].callbacks[spec.captureProp], "capture", path[i]);
      }
      if (!carrier.isStopped()) {
        for (const name of Object.keys(target.callbacks)) {
          if (name === spec.prop || name === spec.captureProp) {
            carrier.invoke(target.callbacks[name], "target", target);
            if (carrier.isImmediate()) break;
          }
        }
      }
      if (bubbles) {
        for (let i = 1; i < path.length && !carrier.isStopped(); i += 1) {
          carrier.invoke(path[i].callbacks[spec.prop], "bubble", path[i]);
        }
      }
      return carrier.event;
    });

  const moveFocus = (node, reveal = false, cause = "programmatic") =>
    runTurn(() => {
      if (node != null) assertLive(node, "focus target");
      const previous = focused.current;
      focused.current = node ?? null;
      focused.reveal = Boolean(reveal);
      if (previous === node) return;
      if (previous && !invalidated.has(previous)) {
        dispatch(previous, "FocusOut", { cause });
      }
      if (node != null && !invalidated.has(node)) {
        dispatch(node, "FocusIn", { cause });
      }
    });

  const syncGeometryAttachment = region => {
    for (const record of geometry) {
      if (!record.active || !record.onAttachment || nodeRegions.get(record.node) !== region)
        continue;
      const next = attached(record.node);
      if (next === record.attached) continue;
      record.attached = next;
      record.onAttachment(next);
    }
  };

  const installRegionStylesheet = (region, key, css) => {
    if (region.stylesheets.get(key) === css) return;
    region.installs.push(key);
    region.stylesheets.set(key, css);
  };

  const reducedMotionSubscribers = new Set();
  const emitReducedMotion = next => {
    const value = Boolean(next);
    if (reducedMotion === value) return;
    reducedMotion = value;
    runTurn(() => {
      for (const callback of [...reducedMotionSubscribers]) {
        if (reducedMotionSubscribers.has(callback)) callback(value);
      }
    }, false);
  };

  const backgroundBlurSubscribers = new Set();
  const emitBackgroundBlurSupported = next => {
    const value = Boolean(next);
    if (backgroundBlurSupported === value) return;
    backgroundBlurSupported = value;
    runTurn(() => {
      for (const callback of [...backgroundBlurSubscribers]) {
        if (backgroundBlurSubscribers.has(callback)) callback(value);
      }
    }, false);
  };

  const makeBridge = region => ({
    createElement: tag => {
      assertActiveHost("createElement");
      return makeNode(tag, region);
    },
    createPortal: layer => {
      assertActiveHost("createPortal");
      const node = makeNode("portal", region);
      makeLease(portals, { layer, node });
      return node;
    },
    createCanvasScene: () => {
      assertActiveHost("createCanvasScene");
      const token = ++nextCanvasToken;
      const record = {
        token,
        region,
        active: true,
        disposals: 0,
        commands: [],
        transform: [1, 0, 0, 1, 0, 0],
      };
      const handle = {
        get token() {
          return token;
        },
        update(commands, transform) {
          if (!record.active) throw new ReferenceError("canvas scene has been disposed");
          assertActiveHost("canvasScene.update");
          if (commands !== null) record.commands = structuredClone(commands);
          if (
            transform !== undefined &&
            transform.some((value, index) => value !== record.transform[index])
          ) {
            record.transform = [...transform];
          }
        },
        dispose() {
          if (!record.active) return;
          record.active = false;
          record.disposals += 1;
          for (const node of nodes.values()) {
            if (!invalidated.has(node) && node.props.get("canvas") === token)
              node.props.delete("canvas");
          }
        },
      };
      canvasScenes.push(record);
      scenesByHandle.set(handle, record);
      region.scenes?.push(handle);
      return handle;
    },
    registerDismissLayer: (node, callback, options) => {
      assertActiveHost("registerDismissLayer");
      assertLive(node, "dismiss layer target", region);
      if (options.exclude) assertLive(options.exclude, "dismiss exclusion", region);
      const { record, dispose } = makeLease(layers, {
        node,
        callback,
        options,
        updates: [],
        trigger: reason => (record.active ? runTurn(() => record.callback(reason)) : undefined),
      });
      return {
        update: next => {
          assertActiveHost("dismissLayer.update");
          assertLive(node, "dismiss layer target", region);
          if (next.exclude) assertLive(next.exclude, "dismiss exclusion", region);
          record.options = next;
          record.updates.push(next);
        },
        dispose,
      };
    },
    createTextNode: text => {
      assertActiveHost("createTextNode");
      if (typeof text !== "string") throw new TypeError("createTextNode expects a string");
      const node = makeNode("#text", region);
      node.props.set("text", text);
      return node;
    },
    replaceText: (node, text) => {
      assertLive(node, "text node", region);
      if (typeof text !== "string") throw new TypeError("replaceText expects a string");
      node.props.set("text", text);
    },
    isTextNode: node => {
      assertLive(node, "node", region);
      return node.tag === "#text";
    },
    insertNode: (parent, node, anchor) => {
      assertActiveHost("insertNode");
      assertLive(parent, "parent", region);
      assertLive(node, "node", region);
      if (anchor !== undefined && anchor !== null) assertLive(anchor, "anchor", region);
      if (node === anchor) return;
      const previous = parents.get(node);
      if (previous) {
        const index = previous.children.indexOf(node);
        if (index >= 0) previous.children.splice(index, 1);
      }
      parents.set(node, parent);
      if (anchor !== undefined && anchor !== null) {
        const index = parent.children.indexOf(anchor);
        if (index < 0) throw new Error("anchor is not a child of the parent");
        parent.children.splice(index, 0, node);
      } else {
        parent.children.push(node);
      }
      syncGeometryAttachment(region);
    },
    removeNode: (parent, node) => {
      assertLive(parent, "parent", region);
      assertLive(node, "node", region);
      const index = parent.children.indexOf(node);
      if (index >= 0) parent.children.splice(index, 1);
      parents.delete(node);
      pendingRemoval.add(node);
      syncGeometryAttachment(region);
    },
    getParentNode: node => {
      assertLive(node, "node", region);
      return parents.get(node) ?? null;
    },
    getFirstChild: node => {
      assertLive(node, "node", region);
      return node.children[0] ?? null;
    },
    getNextSibling: node => {
      assertLive(node, "node", region);
      const parent = parents.get(node);
      if (!parent) return null;
      const index = parent.children.indexOf(node);
      return index >= 0 ? (parent.children[index + 1] ?? null) : null;
    },
    setProperty: (node, name, value) => {
      assertLive(node, "node", region);
      if (
        stopping &&
        ((name.startsWith("on") && value != null) || (name === "src" && value != null))
      ) {
        if (name.startsWith("on")) registrationsAfterStop.push(name);
        throw new ReferenceError("native session is stopping");
      }
      if (name === "svg" || name === "canvas") {
        if (node.tag !== "vector") throw new TypeError(`${name} is only supported on vector nodes`);
        if (value == null) {
          node.props.delete(name);
        } else if (name === "svg") {
          if (typeof value !== "string") throw new TypeError("svg must be a string");
          node.props.set(name, value);
        } else {
          const record = scenesByHandle.get(value);
          if (!record || record.region !== region)
            throw new TypeError("canvas must be a scene from this region");
          if (!record.active) throw new ReferenceError("canvas scene has been disposed");
          node.props.set(name, record.token);
        }
        return;
      }
      if (name === "class" || name === "__zguiClass") {
        if (value != null && typeof value !== "string")
          throw new TypeError("class must be a string");
        const state = stateFor(node);
        value = value?.trim() ? value : null;
        if (name === "__zguiClass") state.internal = value;
        else state.applied = value;
        const classes = [
          ...new Set(
            `${state.internal ?? ""} ${state.applied ?? ""}`.split(/[\t\n\f\r ]+/).filter(Boolean),
          ),
        ].join(" ");
        if (classes) node.props.set("class", classes);
        else node.props.delete("class");
        return;
      }
      if (
        name === "style" ||
        name === "__zguiStyle" ||
        name === "__zguiInvariantStyle" ||
        name.startsWith("--")
      ) {
        const state = stateFor(node);
        let next;
        try {
          next = normalizeStyle(name.startsWith("--") ? { [name]: value } : value);
        } catch (error) {
          failStyle("STYLE_VALUE_INVALID", error.message);
          return;
        }
        if (name.startsWith("--")) {
          if (value == null) delete state.direct[name];
          else Object.assign(state.direct, next);
        } else if (name === "__zguiStyle") state.defaults = next;
        else if (name === "__zguiInvariantStyle") state.invariants = next;
        else {
          state.application = next;
          state.direct = {};
        }
        commitStyle(node, state);
        return;
      }
      delete node.callbacks[name];
      if (typeof value === "function") {
        node.callbacks[name] = value;
        return;
      }
      if (value === undefined || value === null) node.props.delete(name);
      else node.props.set(name, value);
    },
    installStylesheet: (name, css, target) => {
      assertActiveHost("installStylesheet");
      if (region.retired) throw new ReferenceError("native session is disposed");
      if (target !== undefined) {
        assertLive(target, "stylesheet target", region);
        if (!attached(target)) throw new ReferenceError("native stylesheet target is detached");
      }
      installRegionStylesheet(region, name, css);
    },
    removeStylesheet: name => {
      if (!region.retired) region.stylesheets.delete(name);
    },
    getReducedMotion: () => reducedMotion,
    subscribeReducedMotion: callback => {
      reducedMotionSubscribers.add(callback);
      return () => {
        reducedMotionSubscribers.delete(callback);
      };
    },
    getBackgroundBlurSupported: () => backgroundBlurSupported,
    subscribeBackgroundBlurSupported: callback => {
      backgroundBlurSubscribers.add(callback);
      return () => {
        backgroundBlurSubscribers.delete(callback);
      };
    },
    focus: (node, reveal = false) => {
      assertLive(node, "focus target", region);
      moveFocus(node, reveal);
    },
    trapFocus: (node, wrap, autoFocus, restore) => {
      assertActiveHost("trapFocus");
      assertLive(node, "focus trap target", region);
      return makeLease(traps, { node, wrap, autoFocus, restore }).dispose;
    },
    lockScroll: node => {
      assertActiveHost("lockScroll");
      assertLive(node, "scroll lock target", region);
      return makeLease(locks, { node }).dispose;
    },
    scrollTo: (node, left, top, behavior = "instant") => {
      assertActiveHost("scrollTo");
      assertLive(node, "scrollTo target", region);
      scrollCommands.push({ kind: "scrollTo", node, left, top, behavior });
    },
    scrollBy: (node, left, top, behavior = "instant") => {
      assertActiveHost("scrollBy");
      assertLive(node, "scrollBy target", region);
      scrollCommands.push({ kind: "scrollBy", node, left, top, behavior });
    },
    scrollIntoView: (node, block = "nearest", behavior = "instant") => {
      assertActiveHost("scrollIntoView");
      assertLive(node, "scrollIntoView target", region);
      scrollCommands.push({ kind: "scrollIntoView", node, block, behavior });
    },
    getSelection: node => {
      assertLive(node, "getSelection target", region);
      assertEditable(node, "getSelection");
      const current = selections.get(node);
      return current ? { start: current.start, end: current.end } : null;
    },
    setSelection: (node, start, end = start) => {
      assertActiveHost("setSelection");
      assertLive(node, "setSelection target", region);
      assertEditable(node, "setSelection");
      if (
        typeof start !== "number" ||
        !Number.isFinite(start) ||
        typeof end !== "number" ||
        !Number.isFinite(end)
      ) {
        throw new RangeError("selection offset must be a finite number");
      }
      const text = editableTextOf(node);
      const s = clampUtf16Boundary(text, start);
      const e = Math.max(s, clampUtf16Boundary(text, end));
      selections.set(node, { start: s, end: e });
    },
    selectAll: node => {
      assertActiveHost("selectAll");
      assertLive(node, "selectAll target", region);
      assertEditable(node, "selectAll");
      const text = editableTextOf(node);
      selections.set(node, { start: 0, end: text.length });
    },
    getDocumentSelection: () => null,
    setDocumentSelection: (
      container,
      anchorNode,
      anchorOffset,
      focusNode = anchorNode,
      focusOffset = anchorOffset,
    ) => {
      assertActiveHost("setDocumentSelection");
      assertLive(container, "setDocumentSelection container", region);
      assertLive(anchorNode, "setDocumentSelection anchor", region);
      assertLive(focusNode, "setDocumentSelection focus", region);
      if (!Number.isFinite(anchorOffset) || !Number.isFinite(focusOffset)) {
        throw new RangeError("document selection offset must be finite");
      }
    },
    clearDocumentSelection: () => {
      assertActiveHost("clearDocumentSelection");
    },
    selectDocumentAll: container => {
      assertActiveHost("selectDocumentAll");
      assertLive(container, "selectDocumentAll container", region);
    },
    setWindowTitle: title => {
      assertActiveHost("setWindowTitle");
      if (typeof title !== "string") throw new TypeError("window title must be a string");
      const win = windowStateFor(region);
      win.state = { ...win.state, title };
      windowCommands.push({ op: "setWindowTitle", region, title });
      notifyWindowState(win);
    },
    setWindowFullscreen: fullscreen => {
      assertActiveHost("setWindowFullscreen");
      if (typeof fullscreen !== "boolean") throw new TypeError("fullscreen must be a boolean");
      const win = windowStateFor(region);
      win.state = { ...win.state, fullscreen };
      windowCommands.push({ op: "setWindowFullscreen", region, fullscreen });
      notifyWindowState(win);
    },
    setWindowMaximized: maximized => {
      assertActiveHost("setWindowMaximized");
      if (typeof maximized !== "boolean") throw new TypeError("maximized must be a boolean");
      const win = windowStateFor(region);
      win.state = { ...win.state, maximized };
      windowCommands.push({ op: "setWindowMaximized", region, maximized });
      notifyWindowState(win);
    },
    setWindowMinimized: minimized => {
      assertActiveHost("setWindowMinimized");
      if (typeof minimized !== "boolean") throw new TypeError("minimized must be a boolean");
      const win = windowStateFor(region);
      win.state = { ...win.state, minimized };
      windowCommands.push({ op: "setWindowMinimized", region, minimized });
      notifyWindowState(win);
    },
    setWindowDecorations: decorations => {
      assertActiveHost("setWindowDecorations");
      if (decorations !== "full" && decorations !== "none" && decorations !== "no-title-bar") {
        throw new TypeError("invalid window decorations");
      }
      const win = windowStateFor(region);
      win.state = { ...win.state, decorations };
      windowCommands.push({ op: "setWindowDecorations", region, decorations });
      notifyWindowState(win);
    },
    beginWindowMoveDrag: () => {
      assertActiveHost("beginWindowMoveDrag");
      windowCommands.push({ op: "beginWindowMoveDrag", region });
    },
    closeWindow: (force = false) => {
      assertActiveHost("closeWindow");
      const win = windowStateFor(region);
      windowCommands.push({ op: "closeWindow", region, force: Boolean(force) });
      if (!force && runCloseHandlers(region)) return;
      win.closed = true;
    },
    getWindowState: () => ({ ...windowStateFor(region).state }),
    subscribeWindowState: callback => {
      assertActiveHost("subscribeWindowState");
      const win = windowStateFor(region);
      win.stateSubscribers.add(callback);
      return () => {
        win.stateSubscribers.delete(callback);
      };
    },
    onCloseRequest: callback => {
      assertActiveHost("onCloseRequest");
      const win = windowStateFor(region);
      win.closeHandlers.add(callback);
      return () => {
        win.closeHandlers.delete(callback);
      };
    },
    getWindowRect: node => {
      assertLive(node, "rect target", region);
      queries.rect.push(node);
      return rects.get(node) ?? null;
    },
    getTextContent: node => {
      assertLive(node, "text target", region);
      queries.text.push(node);
      return textOf(node, texts);
    },
    isAnimating: node => {
      assertLive(node, "animation target", region);
      animatingQueries.push(node);
      return animating.get(node) ?? false;
    },
    observeGeometry: (node, kind, callback, onAttachment) => {
      assertActiveHost("observeGeometry");
      assertLive(node, "geometry target", region);
      const { record, dispose } = makeLease(geometry, {
        node,
        kind,
        callback,
        onAttachment,
        attached: onAttachment ? attached(node) : undefined,
        emit: value =>
          record.active && (!onAttachment || record.attached)
            ? runTurn(() => record.callback(value))
            : undefined,
      });
      try {
        onAttachment?.(record.attached);
      } catch (error) {
        dispose();
        throw error;
      }
      return dispose;
    },
    precedes: (first, second) => {
      assertLive(first, "first node", region);
      assertLive(second, "second node", region);
      queries.precedes.push([first, second]);
      if (first === second) return false;
      const rootOf = node => {
        let current = node;
        while (parents.get(current)) current = parents.get(current);
        return current;
      };
      if (rootOf(first) !== rootOf(second)) return false;
      const order = new Map();
      let index = 0;
      const walk = node => {
        order.set(node, index++);
        for (const child of node.children) walk(child);
      };
      walk(rootOf(first));
      return order.get(first) < order.get(second);
    },
    clearRoot: root => {
      assertLive(root, "root", region);
      clearCalls.push(root);
      for (const child of root.children) {
        parents.delete(child);
        pendingRemoval.add(child);
      }
      root.children.length = 0;
      syncGeometryAttachment(region);
    },
  });

  const host = makeBridge(primary);
  const surfaces = new Map();
  const shellStylesheets = new Map();
  const forEachRegion = apply => {
    apply(primary);
    for (const { region } of surfaces.values()) apply(region);
  };
  const validateStylesheetName = name => {
    if (
      typeof name !== "string" ||
      name.length === 0 ||
      name.length > 128 ||
      /[^a-zA-Z0-9_.:-]/.test(name)
    ) {
      throw new TypeError("stylesheet names use 1-128 ASCII bytes from [a-zA-Z0-9_.:-]");
    }
  };
  const layerIds = new Map();
  const popupIds = new Map();
  const windowIds = new Map();
  const lockIds = new Map();
  const lockSubscribers = new Map();
  const sessionLockLog = [];
  let lockState = "unlocked";
  const shellTasks = [];
  const shellLog = [];
  const monitorSubscribers = new Set();
  const normalizeMonitors = list =>
    structuredClone(list).map(monitor => ({
      ...monitor,
      logicalWidth: monitor.logicalWidth ?? monitor.width / monitor.scale,
      logicalHeight: monitor.logicalHeight ?? monitor.height / monitor.scale,
    }));
  let monitorSnapshot = normalizeMonitors(monitors);
  let nextLayerId = 0;
  const requireShell = () => {
    if (!shellSurfaces)
      throw Object.assign(new Error("shell surfaces require capabilities.shellSurfaces"), {
        code: "E_CAPABILITY",
      });
  };
  const requireAcyclicPopupParent = (name, parent) => {
    if (name === parent) throw new TypeError("a popup cannot parent itself");
    let ancestor = parent;
    while (ancestor !== undefined) {
      if (ancestor === name) throw new TypeError("popup parent chain contains a cycle");
      let next;
      for (const popup of popupIds.values()) {
        if (!popup.closing && popup.name === ancestor) {
          next = popup.placement.parent;
          break;
        }
      }
      ancestor = next;
    }
  };
  const requireLock = () => {
    if (!sessionLock)
      throw Object.assign(new Error("session lock requires capabilities.sessionLock"), {
        code: "E_CAPABILITY",
      });
  };
  const setLockState = next => {
    lockState = next;
    for (const id of [...lockSubscribers.keys()]) {
      lockSubscribers.get(id)?.(next);
    }
  };
  const present = surface => {
    if (stopping) return false;
    if (surface.window) return true;
    if (surface.popup) {
      const parent = surfaces.get(surface.placement.parent);
      return parent && !parent.lock && !parent.closing;
    }
    return (
      (!surface.lock || lockState === "locked") &&
      (surface.placement.output === undefined ||
        monitorSnapshot.some(monitor => monitor.id === surface.placement.output))
    );
  };
  const attachSurface = surface => {
    if (surface.closing || surface.root || !present(surface)) return;
    const region = { stylesheets: new Map(), installs: [], scenes: [] };
    for (const { key, css } of shellStylesheets.values()) installRegionStylesheet(region, key, css);
    surface.region = region;
    surface.root = makeNode("root", region);
    surface.bridge = makeBridge(region);
    surfaces.set(surface.name, surface);
    surface.dispose = surface.attach(surface.root, surface.bridge);
  };
  const detachSurface = surface => {
    if (!surface.root) return;
    const region = surface.region;
    const previous = inCleanup;
    inCleanup = true;
    try {
      surface.dispose?.();
    } finally {
      inCleanup = previous;
      cleanupJobsPending ||= microtasks.length > 0;
      region.retired = true;
      if (surfaces.get(surface.name) === surface) surfaces.delete(surface.name);
      const doomed = new Set([...nodes.values()].filter(node => nodeRegions.get(node) === region));
      invalidate(doomed);
      for (const scene of region.scenes) scene.dispose();
      region.stylesheets.clear();
      if (doomed.has(focused.current)) focused.current = null;
      surface.root = surface.bridge = surface.region = surface.dispose = undefined;
    }
  };
  const detachTree = (surface, mode) => {
    if (!surface.root) return;
    for (const child of popupIds.values()) {
      if (child.root && child.placement.parent === surface.name) {
        if (mode === "compositor-dismissed") child.suppressed = true;
        detachTree(child, mode);
      }
    }
    detachSurface(surface);
  };
  const attachPopups = () => {
    let changed;
    do {
      changed = false;
      for (const popup of popupIds.values()) {
        if (!popup.closing && !popup.suppressed && !popup.root && present(popup)) {
          attachSurface(popup);
          changed = true;
        }
      }
    } while (changed);
  };

  const shell = {
    getSessionGeneration: () => generation,
    ...(shellReducedMotion
      ? {
          getReducedMotion: host.getReducedMotion,
          subscribeReducedMotion: host.subscribeReducedMotion,
        }
      : {}),
    getBackgroundBlurSupported: () => backgroundBlurSupported,
    subscribeBackgroundBlurSupported(callback) {
      backgroundBlurSubscribers.add(callback);
      return () => {
        backgroundBlurSubscribers.delete(callback);
      };
    },
    installStylesheet(name, css) {
      requireShellWorkAllowed("installStylesheet");
      validateStylesheetName(name);
      if (typeof css !== "string") throw new TypeError("stylesheet CSS must be a string");
      const existing = shellStylesheets.get(name);
      if (existing?.css === css) return;
      // Keep region-local keys inspectable as strings without sharing their namespace.
      const key = existing?.key ?? Symbol(`shell:${name}`);
      shellStylesheets.set(name, { key, css });
      forEachRegion(region => installRegionStylesheet(region, key, css));
    },
    removeStylesheet(name) {
      validateStylesheetName(name);
      const sheet = shellStylesheets.get(name);
      if (!sheet) return;
      shellStylesheets.delete(name);
      forEachRegion(region => {
        region.stylesheets.delete(sheet.key);
      });
    },
    requestSessionLock(callback) {
      requireShellWorkAllowed("requestSessionLock");
      requireLock();
      if (!Number.isSafeInteger(nextLockRequestId))
        throw new RangeError("session lock request identifiers exhausted");
      const id = nextLockRequestId++;
      lockSubscribers.set(id, callback);
      try {
        callback(lockState);
      } catch (error) {
        lockSubscribers.delete(id);
        throw error;
      }
      shellTasks.push(() => {
        if (stopping || lockState === "locked") return;
        sessionLockLog.push("lock");
        setLockState("pending");
        if (sessionLock === "finished") {
          sessionLockLog.push("destroy");
          setLockState("refused");
        } else {
          sessionLockLog.push("locked");
          setLockState("locked");
          for (const surface of lockIds.values()) attachSurface(surface);
        }
      });
      const handle = {
        unlock() {
          if (stopping) throw new TypeError("session is stopping");
          if (!lockSubscribers.has(id))
            throw new ReferenceError("session lock handle was unsubscribed");
          if (lockState !== "locked") throw new TypeError("session is not locked");
          requireShellWorkAllowed("unlock");
          shellTasks.push(() => {
            if (stopping || lockState !== "locked") return;
            sessionLockLog.push("unlock", "sync");
            for (const surface of lockIds.values()) detachSurface(surface);
            setLockState("unlocked");
          });
        },
        unsubscribe: () => {
          lockSubscribers.delete(id);
        },
      };
      return Object.defineProperty(handle, "id", {
        value: id,
        enumerable: true,
        writable: false,
        configurable: false,
      });
    },
    openLock(name, output, attach) {
      requireShellWorkAllowed("openLock");
      requireLock();
      if (name === "main" || !/^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$/.test(name)) {
        throw new TypeError("invalid or reserved shell surface name");
      }
      if (
        [
          ...layerIds.values(),
          ...lockIds.values(),
          ...popupIds.values(),
          ...windowIds.values(),
        ].some(
          surface =>
            !surface.closing &&
            (surface.name === name || (surface.lock && surface.placement.output === output)),
        )
      ) {
        throw new TypeError("duplicate lock surface name or output");
      }
      const id = ++nextLayerId;
      const surface = { id, lock: true, name, placement: { output }, attach };
      lockIds.set(id, surface);
      shellTasks.push(() => attachSurface(surface));
      return id;
    },
    closeLock(id) {
      const surface = lockIds.get(id);
      if (!surface || surface.closing) return;
      surface.closing = true;
      shellTasks.push(() => {
        try {
          detachSurface(surface);
        } finally {
          lockIds.delete(id);
        }
      });
    },
    subscribeMonitors(callback) {
      requireShellWorkAllowed("subscribeMonitors");
      monitorSubscribers.add(callback);
      callback(structuredClone(monitorSnapshot));
      return () => {
        monitorSubscribers.delete(callback);
      };
    },
    openLayer(name, placement, attach) {
      requireShellWorkAllowed("openLayer");
      requireShell();
      if (name === "main" || !/^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$/.test(name))
        throw new TypeError("invalid or reserved shell surface name");
      if (
        [
          ...layerIds.values(),
          ...lockIds.values(),
          ...popupIds.values(),
          ...windowIds.values(),
        ].some(surface => !surface.closing && surface.name === name)
      ) {
        throw new TypeError(`duplicate shell surface: ${name}`);
      }
      const id = ++nextLayerId;
      const surface = {
        id,
        name,
        placement: structuredClone(placement),
        attach,
        inputRegion: "full",
      };
      layerIds.set(id, surface);
      shellLog.push({ op: "open", id, name, placement: surface.placement });
      shellTasks.push(() => attachSurface(surface));
      return id;
    },
    updateLayer(id, placement) {
      assertActiveHost("updateLayer");
      const surface = layerIds.get(id);
      if (!surface || surface.closing) throw new ReferenceError("closed shell surface");
      if (surface.placement.output !== placement.output)
        throw new TypeError("changing an output requires a new shell surface");
      surface.placement = structuredClone(placement);
      shellLog.push({ op: "update", id, placement: surface.placement });
    },
    closeLayer(id) {
      const surface = layerIds.get(id);
      if (!surface || surface.closing) return;
      surface.closing = true;
      shellLog.push({ op: "close", id });
      shellTasks.push(() => {
        try {
          detachTree(surface, "owner-closed");
        } finally {
          layerIds.delete(id);
        }
      });
    },
    setInputRegion(id, region) {
      assertActiveHost("setInputRegion");
      const surface = layerIds.get(id) ?? popupIds.get(id);
      if (!surface || surface.closing) throw new ReferenceError("closed shell surface");
      surface.inputRegion = structuredClone(region);
      shellLog.push({ op: "input", id, region: surface.inputRegion });
    },
    setBlurRegion(id, region) {
      assertActiveHost("setBlurRegion");
      if (lockIds.has(id)) {
        throw new TypeError("session lock surfaces do not support blurRegion");
      }
      const surface = layerIds.get(id) ?? popupIds.get(id) ?? windowIds.get(id);
      if (!surface || surface.closing) throw new ReferenceError("closed shell surface");
      const cloned =
        typeof region === "object" &&
        region !== null &&
        !Array.isArray(region) &&
        Array.isArray(region.nodes)
          ? { nodes: [...region.nodes], minOpacity: region.minOpacity }
          : structuredClone(region);
      surface.blurRegion = cloned;
      shellLog.push({ op: "blur", id, region: surface.blurRegion });
    },
    openPopup(name, placement, attach, dismiss) {
      requireShellWorkAllowed("openPopup");
      requireShell();
      if (name === "main" || !/^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$/.test(name)) {
        throw new TypeError("invalid popup name");
      }
      requireAcyclicPopupParent(name, placement.parent);
      if (
        [
          ...layerIds.values(),
          ...lockIds.values(),
          ...popupIds.values(),
          ...windowIds.values(),
        ].some(surface => !surface.closing && surface.name === name)
      ) {
        throw new TypeError(`duplicate shell surface: ${name}`);
      }
      const id = ++nextLayerId;
      const surface = {
        id,
        popup: true,
        name,
        placement: structuredClone(placement),
        attach,
        dismiss,
      };
      popupIds.set(id, surface);
      return id;
    },
    updatePopup(id, placement) {
      assertActiveHost("updatePopup");
      const surface = popupIds.get(id);
      if (!surface || surface.closing) throw new ReferenceError("closed popup surface");
      if (surface.placement.parent !== placement.parent) {
        requireAcyclicPopupParent(surface.name, placement.parent);
        throw new TypeError("changing a popup parent requires a new shell surface");
      }
      surface.placement = structuredClone(placement);
    },
    closePopup(id) {
      const surface = popupIds.get(id);
      if (!surface || surface.closing) return;
      surface.closing = true;
      shellTasks.push(() => {
        try {
          detachTree(surface, "owner-closed");
        } finally {
          popupIds.delete(id);
        }
      });
    },
    openWindow(name, options, attach, onClose) {
      requireShellWorkAllowed("openWindow");
      requireShell();
      if (name === "main" || !/^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$/.test(name)) {
        throw new TypeError("invalid or reserved window surface name");
      }
      if (
        [
          ...layerIds.values(),
          ...lockIds.values(),
          ...popupIds.values(),
          ...windowIds.values(),
        ].some(surface => !surface.closing && surface.name === name)
      ) {
        throw new TypeError(`duplicate shell surface: ${name}`);
      }
      const id = ++nextLayerId;
      const surface = {
        id,
        window: true,
        name,
        options: structuredClone(options),
        attach,
        onClose,
      };
      windowIds.set(id, surface);
      shellLog.push({ op: "openWindow", id, name, options: surface.options });
      shellTasks.push(() => attachSurface(surface));
      return id;
    },
    updateWindow(id, options) {
      assertActiveHost("updateWindow");
      const surface = windowIds.get(id);
      if (!surface || surface.closing) throw new ReferenceError("closed window surface");
      if (
        surface.options.appId !== options.appId ||
        surface.options.transparent !== options.transparent
      ) {
        throw new TypeError("changing window appId or transparency requires a new shell surface");
      }
      surface.options = structuredClone(options);
      shellLog.push({ op: "updateWindow", id, options: surface.options });
    },
    closeWindow(id) {
      const surface = windowIds.get(id);
      if (!surface || surface.closing) return;
      surface.closing = true;
      shellLog.push({ op: "closeWindow", id });
      shellTasks.push(() => {
        try {
          detachTree(surface, "owner-closed");
        } finally {
          windowIds.delete(id);
        }
      });
    },
  };
  const requestClose = (name = "main") => {
    if (name === "main") {
      const vetoed = runTurn(() => runCloseHandlers(primary), false);
      if (!vetoed) windowStateFor(primary).closed = true;
      return !vetoed;
    }
    const surface = surfaces.get(name);
    if (!surface?.window || surface.closing)
      throw new ReferenceError(`no live window named ${name}`);
    const vetoed = runTurn(() => runCloseHandlers(surface.region), false);
    if (vetoed) return false;
    windowClose(name);
    return true;
  };
  const windowClose = name => {
    const surface = surfaces.get(name);
    if (!surface?.window || surface.closing)
      throw new ReferenceError(`no live window named ${name}`);
    runTurn(() => {});
    surface.closing = true;
    try {
      runTurn(() => detachTree(surface, "compositor-dismissed"), false);
      runTurn(() => {});
      runTurn(surface.onClose, false);
    } finally {
      windowIds.delete(surface.id);
    }
  };
  const popupDone = name => {
    const surface = surfaces.get(name);
    if (!surface?.popup || surface.closing) throw new ReferenceError(`no live popup named ${name}`);
    runTurn(() => {});
    surface.closing = true;
    try {
      runTurn(() => detachTree(surface, "compositor-dismissed"), false);
      runTurn(() => {});
      runTurn(surface.dismiss, false);
    } finally {
      popupIds.delete(surface.id);
    }
  };

  const emitMonitors = next => {
    monitorSnapshot = normalizeMonitors(next);
    // Reconcile output loss before the JS job checkpoint, as the native host does.
    runTurn(() => {
      for (const callback of [...monitorSubscribers]) {
        if (monitorSubscribers.has(callback)) callback(structuredClone(monitorSnapshot));
      }
    }, false);
    runTurn(() => {
      for (const surface of [...layerIds.values(), ...lockIds.values()]) {
        if (surface.closing || !present(surface)) detachTree(surface, "owner-closed");
      }
    }, false);
    shellTasks.push(() => {
      for (const surface of layerIds.values()) attachSurface(surface);
      for (const surface of lockIds.values()) attachSurface(surface);
    });
  };

  const flush = () => {
    runTurn(() => {});
    do {
      for (const task of shellTasks.splice(0, shellTasks.length)) runTurn(task);
      runTurn(attachPopups);
    } while (shellTasks.length);
  };

  const attached = node => {
    if (invalidated.has(node)) return false;
    let current = node;
    while (current) {
      if (current.tag === "root") return true;
      current = parents.get(current);
    }
    return false;
  };

  const byTestId = testId =>
    [...nodes.values()].find(node => node.props.get("testId") === testId && attached(node));

  hostMicrotasks.set(host, microtasks);
  hostMicrotasks.set(shell, microtasks);

  return {
    host,
    nodes,
    parents,
    stylesheets,
    installs,
    makeNode,
    byTestId,
    focused,
    rects,
    texts,
    shell,
    surfaces,
    shellLog,
    emitMonitors,
    emitReducedMotion,
    reducedMotionSubscribers,
    emitBackgroundBlurSupported,
    backgroundBlurSubscribers,
    monitorSubscribers,
    popupDone,
    windowClose,
    requestClose,
    windowCommands,
    windowState: (name = "main") => ({
      ...windowStateFor(name === "main" ? primary : surfaces.get(name)?.region).state,
    }),
    windowClosed: (name = "main") =>
      windowStateFor(name === "main" ? primary : surfaces.get(name)?.region).closed,
    sessionLockState: () => lockState,
    sessionLockLog,
    animating,
    animatingQueries,
    queries,
    portals,
    traps,
    locks,
    scrollCommands,
    layers,
    geometry,
    canvasScenes,
    clearCalls,
    invalidated,
    registrationsAfterStop,
    styleDiagnostics,
    dispatch,
    createEvent,
    runTurn,
    endTurn,
    flush,
    focus: (node, options) =>
      moveFocus(node, options?.reveal ?? false, options?.cause ?? "programmatic"),
    beginStop: () => {
      stopping = true;
      forEachRegion(region => {
        for (const { key } of shellStylesheets.values()) region.stylesheets.delete(key);
      });
      shellStylesheets.clear();
      lockSubscribers.clear();
      for (const surface of popupIds.values()) {
        surface.closing = true;
        detachTree(surface, "owner-closed");
      }
      popupIds.clear();
      for (const surface of windowIds.values()) {
        surface.closing = true;
        detachTree(surface, "owner-closed");
      }
      windowIds.clear();
      for (const surface of lockIds.values()) {
        surface.closing = true;
        detachSurface(surface);
      }
      lockIds.clear();
    },
    isStopping: () => stopping,
  };
}

export function createClock(t, harness) {
  let now = 0;
  let nextId = 0;
  const timers = new Map();
  const frames = new Map();
  t.mock.method(globalThis, "setTimeout", (callback, delay = 0) => {
    const id = ++nextId;
    timers.set(id, { at: now + Math.max(0, delay), callback });
    return id;
  });
  t.mock.method(globalThis, "clearTimeout", id => {
    timers.delete(id);
  });
  const previousFrame = Object.getOwnPropertyDescriptor(globalThis, "requestAnimationFrame");
  const previousCancel = Object.getOwnPropertyDescriptor(globalThis, "cancelAnimationFrame");
  globalThis.requestAnimationFrame = callback => {
    const id = ++nextId;
    frames.set(id, callback);
    return id;
  };
  globalThis.cancelAnimationFrame = id => {
    frames.delete(id);
  };
  t.after(() => {
    if (previousFrame) Object.defineProperty(globalThis, "requestAnimationFrame", previousFrame);
    else delete globalThis.requestAnimationFrame;
    if (previousCancel) Object.defineProperty(globalThis, "cancelAnimationFrame", previousCancel);
    else delete globalThis.cancelAnimationFrame;
  });
  const invoke = fn => (harness ? harness.runTurn(fn) : fn());
  const frame = () => {
    harness?.flush();
    const due = [...frames.entries()];
    for (const [id] of due) frames.delete(id);
    for (const [, callback] of due) invoke(() => callback(now));
  };
  const advance = ms => {
    harness?.flush();
    const target = now + ms;
    for (;;) {
      let earliest;
      for (const [id, timer] of timers) {
        if (
          timer.at <= target &&
          (!earliest ||
            timer.at < earliest.timer.at ||
            (timer.at === earliest.timer.at && id < earliest.id))
        ) {
          earliest = { id, timer };
        }
      }
      if (!earliest) break;
      timers.delete(earliest.id);
      now = earliest.timer.at;
      invoke(earliest.timer.callback);
    }
    now = target;
  };
  const settle = () => {
    for (let i = 0; i < 4; i += 1) {
      frame();
      advance(1000);
    }
  };
  return { timers, frames, frame, advance, settle };
}

export async function moduleFor(t, source) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "zgui-components-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const entry = path.join(directory, "main.tsx");
  await writeFile(entry, source);
  const result = await bundle(entry, { appDirectory: directory, hostEntry: false });
  for (const [name, bytes] of result.files) await writeFile(path.join(directory, name), bytes);
  return import(pathToFileURL(path.join(directory, "app.mjs")).href);
}
