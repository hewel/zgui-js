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
  if (type === "wheel") {
    return {
      position: { x: 0, y: 0 }, pointerId: "1", pointerKind: "mouse",
      delta: { x: 0, y: 0, unit: "lines" }, scrollPhase: "discrete",
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
    get defaultPrevented() { return prevented; },
    get propagationStopped() { return stopped; },
    get immediatePropagationStopped() { return immediate; },
    preventDefault() { requireActive(isActive); prevented = true; },
    stopPropagation() { requireActive(isActive); stopped = true; },
    stopImmediatePropagation() { requireActive(isActive); immediate = true; stopped = true; },
  });
  const requireActive = isActive => {
    if (!isActive()) throw new ReferenceError("expired-dispatch: native event cancellation is synchronous");
  };
  return {
    spec,
    get event() { return snapshot(currentTarget, phase, () => false); },
    invoke(handler, nextPhase, nextCurrent) {
      phase = nextPhase;
      currentTarget = nextCurrent;
      if (!handler) return;
      let active = true;
      try { handler(snapshot(currentTarget, phase, () => active)); }
      finally { active = false; }
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

export function createHost({ strictStyles = false, reducedMotion = false, monitors = [], sessionLock } = {}) {
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
    const unitless = new Set(["opacity", "flex-grow", "flex-shrink", "font-weight", "line-height", "z-index", "order", "animation-iteration-count", "scale"]);
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

  const syncGeometryAttachment = region => {
    for (const record of geometry) {
      if (!record.active || !record.onAttachment || nodeRegions.get(record.node) !== region) continue;
      const next = attached(record.node);
      if (next === record.attached) continue;
      record.attached = next;
      record.onAttachment(next);
    }
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
      const record = { token, region, active: true, disposals: 0, commands: [], transform: [1, 0, 0, 1, 0, 0] };
      const handle = {
        get token() { return token; },
        update(commands, transform) {
          if (!record.active) throw new ReferenceError("canvas scene has been disposed");
          assertActiveHost("canvasScene.update");
          if (commands !== null) record.commands = structuredClone(commands);
          if (transform !== undefined && transform.some((value, index) => value !== record.transform[index])) {
            record.transform = [...transform];
          }
        },
        dispose() {
          if (!record.active) return;
          record.active = false;
          record.disposals += 1;
          for (const node of nodes.values()) {
            if (!invalidated.has(node) && node.props.get("canvas") === token) node.props.delete("canvas");
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
        trigger: reason => record.active ? runTurn(() => record.callback(reason)) : undefined,
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
      return index >= 0 ? parent.children[index + 1] ?? null : null;
    },
    setProperty: (node, name, value) => {
      assertLive(node, "node", region);
      if (stopping && ((name.startsWith("on") && value != null) || (name === "src" && value != null))) {
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
          if (!record || record.region !== region) throw new TypeError("canvas must be a scene from this region");
          if (!record.active) throw new ReferenceError("canvas scene has been disposed");
          node.props.set(name, record.token);
        }
        return;
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
      delete node.callbacks[name];
      if (typeof value === "function") {
        node.callbacks[name] = value;
        return;
      }
      if (value === undefined || value === null) node.props.delete(name);
      else node.props.set(name, value);
    },
    installStylesheet: (name, css) => {
      assertActiveHost("installStylesheet");
      if (region.stylesheets.get(name) === css) return;
      region.installs.push(name);
      region.stylesheets.set(name, css);
    },
    removeStylesheet: name => { region.stylesheets.delete(name); },
    getReducedMotion: () => reducedMotion,
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
        emit: value => record.active && (!onAttachment || record.attached)
          ? runTurn(() => record.callback(value)) : undefined,
      });
      try { onAttachment?.(record.attached); }
      catch (error) { dispose(); throw error; }
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
  const layerIds = new Map();
  const lockIds = new Map();
  const lockSubscribers = new Set();
  const sessionLockLog = [];
  let lockState = "unlocked";
  const shellTasks = [];
  const shellLog = [];
  const monitorSubscribers = new Set();
  let monitorSnapshot = structuredClone(monitors);
  let nextLayerId = 0;
  const requireLock = () => {
    if (!sessionLock) throw Object.assign(new Error("session lock requires capabilities.sessionLock"), { code: "E_CAPABILITY" });
  };
  const setLockState = next => {
    lockState = next;
    for (const callback of [...lockSubscribers]) {
      if (lockSubscribers.has(callback)) callback(next);
    }
  };
  const present = surface => (!surface.lock || lockState === "locked")
    && (surface.placement.output === undefined || monitorSnapshot.some(monitor => monitor.id === surface.placement.output));
  const attachSurface = surface => {
    if (surface.closing || surface.root || !present(surface)) return;
    const region = { stylesheets: new Map(), installs: [], scenes: [] };
    surface.region = region;
    surface.root = makeNode("root", region);
    surface.bridge = makeBridge(region);
    surfaces.set(surface.name, surface);
    surface.dispose = surface.attach(surface.root, surface.bridge);
  };
  const detachSurface = surface => {
    if (!surface.root) return;
    const region = surface.region;
    try { surface.dispose?.(); }
    finally {
      if (surfaces.get(surface.name) === surface) surfaces.delete(surface.name);
      const doomed = new Set([...nodes.values()].filter(node => nodeRegions.get(node) === region));
      invalidate(doomed);
      for (const scene of region.scenes) scene.dispose();
      region.stylesheets.clear();
      if (doomed.has(focused.current)) focused.current = null;
      surface.root = surface.bridge = surface.region = surface.dispose = undefined;
    }
  };

  const shell = {
    requestSessionLock(callback) {
      assertActiveHost("requestSessionLock");
      requireLock();
      callback(lockState);
      lockSubscribers.add(callback);
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
      return {
        unlock() {
          if (stopping) throw new TypeError("session is stopping");
          if (!lockSubscribers.has(callback)) throw new ReferenceError("session lock handle was unsubscribed");
          if (lockState !== "locked") throw new TypeError("session is not locked");
          shellTasks.push(() => {
            if (stopping || lockState !== "locked") return;
            sessionLockLog.push("unlock", "sync");
            for (const surface of lockIds.values()) detachSurface(surface);
            setLockState("unlocked");
          });
        },
        unsubscribe: () => { lockSubscribers.delete(callback); },
      };
    },
    openLock(name, output, attach) {
      assertActiveHost("openLock");
      requireLock();
      if (name === "main" || !/^[a-z0-9][a-z0-9_.-]{0,127}$/.test(name)) {
        throw new TypeError("invalid or reserved shell surface name");
      }
      if ([...layerIds.values(), ...lockIds.values()].some(surface =>
        !surface.closing && (surface.name === name || (surface.lock && surface.placement.output === output)))) {
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
        try { detachSurface(surface); }
        finally { lockIds.delete(id); }
      });
    },
    subscribeMonitors(callback) {
      assertActiveHost("subscribeMonitors");
      monitorSubscribers.add(callback);
      callback(structuredClone(monitorSnapshot));
      return () => { monitorSubscribers.delete(callback); };
    },
    openLayer(name, placement, attach) {
      assertActiveHost("openLayer");
      if (name === "main" || !/^[a-zA-Z0-9_-]{1,64}$/.test(name)) throw new TypeError("invalid or reserved shell surface name");
      if ([...layerIds.values(), ...lockIds.values()].some(surface => !surface.closing && surface.name === name)) {
        throw new TypeError(`duplicate shell surface: ${name}`);
      }
      const id = ++nextLayerId;
      const surface = { id, name, placement: structuredClone(placement), attach, inputRegion: "full" };
      layerIds.set(id, surface);
      shellLog.push({ op: "open", id, name, placement: surface.placement });
      shellTasks.push(() => attachSurface(surface));
      return id;
    },
    updateLayer(id, placement) {
      assertActiveHost("updateLayer");
      const surface = layerIds.get(id);
      if (!surface || surface.closing) throw new ReferenceError("closed shell surface");
      if (surface.placement.output !== placement.output) throw new TypeError("changing an output requires a new shell surface");
      surface.placement = structuredClone(placement);
      shellLog.push({ op: "update", id, placement: surface.placement });
    },
    closeLayer(id) {
      const surface = layerIds.get(id);
      if (!surface || surface.closing) return;
      surface.closing = true;
      shellLog.push({ op: "close", id });
      shellTasks.push(() => {
        try { detachSurface(surface); }
        finally { layerIds.delete(id); }
      });
    },
    setInputRegion(id, region) {
      assertActiveHost("setInputRegion");
      const surface = layerIds.get(id);
      if (!surface || surface.closing) throw new ReferenceError("closed shell surface");
      surface.inputRegion = structuredClone(region);
      shellLog.push({ op: "input", id, region: surface.inputRegion });
    },
  };

  const emitMonitors = next => runTurn(() => {
    monitorSnapshot = structuredClone(next);
    for (const callback of [...monitorSubscribers]) {
      if (monitorSubscribers.has(callback)) callback(structuredClone(monitorSnapshot));
    }
    shellTasks.push(() => {
      for (const surface of [...layerIds.values(), ...lockIds.values()]) {
        if (surface.closing || !present(surface)) detachSurface(surface);
      }
      for (const surface of layerIds.values()) attachSurface(surface);
      for (const surface of lockIds.values()) attachSurface(surface);
    });
  });

  const flush = () => {
    runTurn(() => {});
    while (shellTasks.length) {
      for (const task of shellTasks.splice(0, shellTasks.length)) runTurn(task);
    }
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
    host, nodes, parents, stylesheets, installs, makeNode, byTestId, focused, rects, texts,
    shell, surfaces, shellLog, emitMonitors, monitorSubscribers,
    sessionLockState: () => lockState, sessionLockLog,
    animating, animatingQueries, queries, portals, traps, locks, layers, geometry,
    canvasScenes,
    clearCalls, invalidated, registrationsAfterStop, styleDiagnostics,
    dispatch, createEvent, runTurn, endTurn,
    flush,
    focus: (node, options) => moveFocus(node, options?.reveal ?? false, options?.cause ?? "programmatic"),
    beginStop: () => {
      stopping = true;
      lockSubscribers.clear();
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
