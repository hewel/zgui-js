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
    return { prop: "onFocus", captureProp: "onFocusCapture", type: lower === "focus" ? "focusin" : lower, bubbles: true };
  }
  if (lower === "blur" || lower === "focusout") {
    return { prop: "onBlur", captureProp: "onBlurCapture", type: lower === "blur" ? "focusout" : lower, bubbles: true };
  }
  const pascal = stem.charAt(0).toUpperCase() + stem.slice(1);
  return { prop: `on${pascal}`, captureProp: `on${pascal}Capture`, type: lower, bubbles: !BUBBLE_EXEMPT.has(lower) };
}

function eventFamilyDefaults(type, extras) {
  if (type === "keydown" || type === "keyup") {
    return {
      key: null, character: null, keyWithoutModifiers: extras.key ?? null,
      deadKey: null, code: null, physicalCode: null, location: "standard", repeat: false,
    };
  }
  if (type.startsWith("pointer") || type === "contextmenu") {
    return {
      position: { x: 0, y: 0 }, pointerId: "1", pointerKind: "mouse", primary: true, button: "primary",
      pressure: null, coalesced: [], activateNow() {}, capturePointer() {}, releasePointer() {},
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
  const event = {
    type,
    timestamp: extras.timestamp ?? 0,
    target,
    modifiers,
    ...eventFamilyDefaults(type, extras),
    ...extras,
    get phase() { return phase; },
    get currentTarget() { return currentTarget; },
    get defaultPrevented() { return prevented; },
    get propagationStopped() { return stopped; },
    get immediatePropagationStopped() { return immediate; },
    preventDefault() { prevented = true; },
    stopPropagation() { stopped = true; },
    stopImmediatePropagation() { immediate = true; stopped = true; },
  };
  return {
    spec,
    event,
    setState(nextPhase, nextCurrent) {
      phase = nextPhase;
      currentTarget = nextCurrent;
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
  const queue = globalThis.__zgui ? hostMicrotasks.get(globalThis.__zgui) : undefined;
  if (queue) queue.push(fn);
  else nativeQueueMicrotask(fn);
};

export function createHost({ strictStyles = false, reducedMotion = false } = {}) {
  let nextId = 0;
  let stopping = false;
  let turnDepth = 0;
  const microtasks = [];
  const drainMicrotasks = () => {
    while (microtasks.length) {
      const batch = microtasks.splice(0, microtasks.length);
      for (const task of batch) task();
    }
  };
  const nodes = new Map();
  const parents = new Map();
  const stylesheets = new Map();
  const installs = [];
  const focused = { current: null, reveal: false };
  const rects = new Map();
  const texts = new Map();
  const animating = new Map();
  const invalidated = new Set();
  const pendingRemoval = new Set();
  const portals = [];
  const traps = [];
  const locks = [];
  const layers = [];
  const geometry = [];
  const animatingQueries = [];
  const queries = { rect: [], text: [], precedes: [], animating: animatingQueries };
  const clearCalls = [];
  const registrationsAfterStop = [];
  const leaseGroups = [portals, traps, locks, layers, geometry];
  const styleDiagnostics = [];
  const styling = new WeakMap();
  const stateFor = node => {
    if (!styling.has(node)) styling.set(node, { aliases: {}, applied: null, internal: null, defaults: {}, application: {}, invariants: {}, direct: {} });
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
        result[declaration.slice(0, colon).trim()] = declaration.slice(colon + 1).trim().replace(/\s*!important$/, "");
      }
      value = result;
    }
    if (typeof value !== "object" || Array.isArray(value)) throw new TypeError("style must be a string or object");
    const result = {};
    const unitless = new Set(["opacity", "flex-grow", "flex-shrink", "font-weight", "line-height", "z-index", "order", "animation-iteration-count"]);
    for (const [key, raw] of Object.entries(value)) {
      const name = key.startsWith("--") ? key : key.replace(/[A-Z]/g, match => `-${match.toLowerCase()}`);
      if (raw == null) continue;
      if (typeof raw === "number" && (!Number.isFinite(raw) || !(name.startsWith("--") || unitless.has(name)))) throw new TypeError(`invalid numeric ${name}`);
      if (typeof raw !== "string" && typeof raw !== "number") throw new TypeError(`invalid ${name}`);
      const text = String(raw);
      if (/\b(?:NaN|Infinity)(?:[a-z%]*)\b|\b(?:url|image-set)\s*\(/i.test(text) || /^(?:invalid|not-a-colour)$/.test(text) || name === "unknown-property" || name === "colour") throw new TypeError(`invalid native declaration ${name}`);
      result[name] = text;
    }
    return result;
  };
  const commitStyle = (node, state) => {
    const styles = { ...state.defaults, ...state.application, ...state.direct, ...state.invariants };
    for (const key of node.props.keys()) if (key.startsWith("--")) node.props.delete(key);
    for (const [key, value] of Object.entries(styles)) if (key.startsWith("--")) node.props.set(key, value);
    if (Object.keys(styles).length) node.props.set("style", styles);
    else node.props.delete("style");
  };

  const assertLive = (node, role = "node") => {
    if (!node || typeof node !== "object" || !nodes.has(node.id)) {
      throw new TypeError(`invalid native ${role}`);
    }
    if (invalidated.has(node)) {
      throw new TypeError(`stale detached native ${role}`);
    }
  };

  const assertActiveHost = op => {
    if (stopping) throw new ReferenceError(`native session is stopping during ${op}`);
  };

  const makeNode = tag => {
    const node = { tag, id: ++nextId, props: new Map(), callbacks: {}, children: [] };
    nodes.set(node.id, node);
    return node;
  };

  const collectSubtree = (root, out = new Set()) => {
    out.add(root);
    for (const child of root.children) collectSubtree(child, out);
    return out;
  };

  const endTurn = () => {
    if (pendingRemoval.size === 0) return;
    const roots = [...pendingRemoval];
    pendingRemoval.clear();
    const doomed = new Set();
    for (const candidate of roots) {
      if (!parents.has(candidate) && candidate.tag !== "root") collectSubtree(candidate, doomed);
    }
    if (doomed.size === 0) return;
    for (const group of leaseGroups) {
      for (const record of group) {
        if (doomed.has(record.node)) record._dispose();
      }
    }
    for (const node of doomed) invalidated.add(node);
  };

  const runTurn = fn => {
    turnDepth += 1;
    try { return fn(); }
    finally {
      try {
        if (turnDepth === 1 && !stopping) drainMicrotasks();
      } finally {
        turnDepth -= 1;
        if (turnDepth === 0) endTurn();
      }
    }
  };

  const dispatch = (target, nameOrType, extras = {}) => runTurn(() => {
    assertLive(target, "event target");
    const path = [];
    for (let current = target; current; current = parents.get(current)) path.push(current);
    const carrier = createEvent(nameOrType, target, extras);
    const { spec, event } = carrier;
    const bubbles = extras.bubbles ?? spec.bubbles;
    for (let i = path.length - 1; i >= 1 && !carrier.isStopped(); i -= 1) {
      carrier.setState("capture", path[i]);
      path[i].callbacks[spec.captureProp]?.(event);
    }
    if (!carrier.isStopped()) {
      carrier.setState("target", target);
      target.callbacks[spec.captureProp]?.(event);
    }
    if (!carrier.isStopped() && !carrier.isImmediate()) {
      carrier.setState("target", target);
      target.callbacks[spec.prop]?.(event);
    }
    if (bubbles) {
      for (let i = 1; i < path.length && !carrier.isStopped(); i += 1) {
        carrier.setState("bubble", path[i]);
        path[i].callbacks[spec.prop]?.(event);
      }
    }
    return event;
  });

  const moveFocus = (node, reveal = false, cause = "programmatic") => runTurn(() => {
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

  const host = {
    createElement: tag => {
      assertActiveHost("createElement");
      return makeNode(tag);
    },
    createPortal: layer => {
      assertActiveHost("createPortal");
      const node = makeNode("portal");
      makeLease(portals, { layer, node });
      return node;
    },
    registerDismissLayer: (node, callback, options) => {
      assertActiveHost("registerDismissLayer");
      assertLive(node, "dismiss layer target");
      const { record, dispose } = makeLease(layers, {
        node,
        callback,
        options,
        updates: [],
        trigger: reason => record.active ? runTurn(() => record.callback(reason)) : undefined,
      });
      return {
        update: next => {
          assertActiveHost("dismissLayer.update");
          assertLive(node, "dismiss layer target");
          record.options = next;
          record.updates.push(next);
        },
        dispose,
      };
    },
    createTextNode: text => {
      assertActiveHost("createTextNode");
      if (typeof text !== "string") throw new TypeError("createTextNode expects a string");
      const node = makeNode("#text");
      node.props.set("text", text);
      return node;
    },
    replaceText: (node, text) => {
      assertLive(node, "text node");
      if (typeof text !== "string") throw new TypeError("replaceText expects a string");
      node.props.set("text", text);
    },
    isTextNode: node => {
      assertLive(node, "node");
      return node.tag === "#text";
    },
    insertNode: (parent, node, anchor) => {
      assertActiveHost("insertNode");
      assertLive(parent, "parent");
      assertLive(node, "node");
      if (anchor !== undefined && anchor !== null) assertLive(anchor, "anchor");
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
    },
    removeNode: (parent, node) => {
      assertLive(parent, "parent");
      assertLive(node, "node");
      const index = parent.children.indexOf(node);
      if (index >= 0) parent.children.splice(index, 1);
      parents.delete(node);
      pendingRemoval.add(node);
    },
    getParentNode: node => {
      assertLive(node, "node");
      return parents.get(node) ?? null;
    },
    getFirstChild: node => {
      assertLive(node, "node");
      return node.children[0] ?? null;
    },
    getNextSibling: node => {
      assertLive(node, "node");
      const parent = parents.get(node);
      if (!parent) return null;
      const index = parent.children.indexOf(node);
      return index >= 0 ? parent.children[index + 1] ?? null : null;
    },
    setProperty: (node, name, value) => {
      assertLive(node, "node");
      if (stopping && ((name.startsWith("on") && value != null) || (name === "src" && value != null))) {
        if (name.startsWith("on")) registrationsAfterStop.push(name);
        throw new ReferenceError("native session is stopping");
      }
      if (name === "class" || name === "className" || name === "__zguiClass") {
        if (value != null && typeof value !== "string") throw new TypeError("class must be a string");
        const state = stateFor(node);
        value = value?.trim() ? value : null;
        if (name === "__zguiClass") state.internal = value;
        else state.aliases[name] = value;
        const conflict = state.aliases.class && state.aliases.className;
        if (!conflict) state.applied = state.aliases.class || state.aliases.className;
        const classes = [...new Set(`${state.internal ?? ""} ${state.applied ?? ""}`.split(/[\t\n\f\r ]+/).filter(Boolean))].join(" ");
        if (classes) node.props.set("class", classes); else node.props.delete("class");
        if (conflict && name !== "__zguiClass") failStyle("CLASS_ALIAS_CONFLICT", "class and className both contribute application tokens");
        return;
      }
      if (name === "style" || name === "__zguiStyle" || name === "__zguiInvariantStyle" || name.startsWith("--")) {
        const state = stateFor(node);
        let next;
        try { next = normalizeStyle(name.startsWith("--") ? { [name]: value } : value); }
        catch (error) { failStyle("STYLE_VALUE_INVALID", error.message); return; }
        if (name.startsWith("--")) {
          if (value == null) delete state.direct[name]; else Object.assign(state.direct, next);
        } else if (name === "__zguiStyle") state.defaults = next;
        else if (name === "__zguiInvariantStyle") state.invariants = next;
        else { state.application = next; state.direct = {}; }
        commitStyle(node, state);
        return;
      }
      if (typeof value === "function") {
        node.callbacks[name] = value;
        return;
      }
      delete node.callbacks[name];
      if (value === undefined || value === null) node.props.delete(name);
      else node.props.set(name, value);
    },
    installStylesheet: (name, css) => {
      assertActiveHost("installStylesheet");
      if (stylesheets.get(name) === css) return;
      installs.push(name);
      stylesheets.set(name, css);
    },
    removeStylesheet: name => { stylesheets.delete(name); },
    getReducedMotion: () => reducedMotion,
    focus: (node, reveal = false) => moveFocus(node, reveal),
    trapFocus: (node, wrap, autoFocus, restore) => {
      assertActiveHost("trapFocus");
      assertLive(node, "focus trap target");
      return makeLease(traps, { node, wrap, autoFocus, restore }).dispose;
    },
    lockScroll: node => {
      assertActiveHost("lockScroll");
      assertLive(node, "scroll lock target");
      return makeLease(locks, { node }).dispose;
    },
    getWindowRect: node => {
      assertLive(node, "rect target");
      queries.rect.push(node);
      return rects.get(node) ?? null;
    },
    getTextContent: node => {
      assertLive(node, "text target");
      queries.text.push(node);
      return textOf(node, texts);
    },
    isAnimating: node => {
      assertLive(node, "animation target");
      animatingQueries.push(node);
      return animating.get(node) ?? false;
    },
    observeGeometry: (node, kind, callback) => {
      assertActiveHost("observeGeometry");
      assertLive(node, "geometry target");
      const { record, dispose } = makeLease(geometry, {
        node,
        kind,
        callback,
        emit: value => record.active ? runTurn(() => record.callback(value)) : undefined,
      });
      return dispose;
    },
    precedes: (first, second) => {
      assertLive(first, "first node");
      assertLive(second, "second node");
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
      assertLive(root, "root");
      clearCalls.push(root);
      for (const child of root.children) {
        parents.delete(child);
        pendingRemoval.add(child);
      }
      root.children.length = 0;
    },
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

  return {
    host, nodes, parents, stylesheets, installs, makeNode, byTestId, focused, rects, texts,
    animating, animatingQueries, queries, portals, traps, locks, layers, geometry,
    clearCalls, invalidated, registrationsAfterStop, styleDiagnostics,
    dispatch, createEvent, runTurn, endTurn,
    flush: () => runTurn(() => {}),
    focus: (node, options) => moveFocus(node, options?.reveal ?? false, options?.cause ?? "programmatic"),
    beginStop: () => { stopping = true; },
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
  t.mock.method(globalThis, "clearTimeout", id => { timers.delete(id); });
  const previousFrame = Object.getOwnPropertyDescriptor(globalThis, "requestAnimationFrame");
  const previousCancel = Object.getOwnPropertyDescriptor(globalThis, "cancelAnimationFrame");
  globalThis.requestAnimationFrame = callback => {
    const id = ++nextId;
    frames.set(id, callback);
    return id;
  };
  globalThis.cancelAnimationFrame = id => { frames.delete(id); };
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
        if (timer.at <= target && (!earliest || timer.at < earliest.timer.at || (timer.at === earliest.timer.at && id < earliest.id))) {
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
