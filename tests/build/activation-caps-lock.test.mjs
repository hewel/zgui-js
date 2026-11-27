import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";
import {
  getCapsLock,
  renderApp,
  requestActivationToken,
  useCapsLock,
} from "../../packages/ui/dist/index.js";

function withHost(options, run) {
  const harness = createHost(options);
  const prevZgui = globalThis.__zgui;
  const prevShell = globalThis.__zguiShell;
  globalThis.__zgui = harness.host;
  globalThis.__zguiShell = harness.shell;
  return Promise.resolve()
    .then(() => run(harness))
    .finally(() => {
      if (prevZgui === undefined) delete globalThis.__zgui;
      else globalThis.__zgui = prevZgui;
      if (prevShell === undefined) delete globalThis.__zguiShell;
      else globalThis.__zguiShell = prevShell;
    });
}

test("requestActivationToken validates options synchronously and normalizes empty appId", async () => {
  await withHost({}, async h => {
    for (const invalid of [
      null,
      false,
      123,
      "org.gnome.Terminal",
      [],
      () => {},
      new Date(),
      new Map(),
      { appId: null },
      { appId: 42 },
      { appId: "bad\u0000null" },
      { appId: "bad\nnewline" },
      { appId: "bad\u007fdel" },
      { appId: "bad\u0085nel" },
      { appId: "lone\ud800surrogate" },
      { appId: "lone\udc00trail" },
      { appId: "a".repeat(257) },
      { appId: "中".repeat(86) }, // 86 * 3 = 258 UTF-8 bytes (> 256)
    ]) {
      assert.throws(
        () => requestActivationToken(invalid),
        TypeError,
        `expected synchronous TypeError for ${String(invalid)}`,
      );
    }

    // Outside a gesture: valid options resolve to null (including 256-byte multi-byte UTF-8 boundary and Object.create(null)).
    const protoNull = Object.create(null);
    protoNull.appId = "中".repeat(85) + "a"; // 255 + 1 = 256 UTF-8 bytes
    assert.equal(await requestActivationToken(protoNull), null);
    assert.equal(await requestActivationToken({ appId: "" }), null);
    assert.equal(await requestActivationToken({ appId: undefined }), null);
    assert.equal(await requestActivationToken(), null);

    // Inside a trusted gesture: single-issuance and "" normalization.
    h.setActivationTokenHandler(({ appId }) => `minted:${appId ?? "<omitted>"}`);
    const root = h.makeNode("root");
    const tokens = [];
    const dispose = renderApp(() => {
      const btn = h.host.createElement("control");
      h.host.setProperty(btn, "testId", "launch");
      h.host.setProperty(btn, "onClick", () => {
        tokens.push(requestActivationToken({ appId: "" }));
        tokens.push(requestActivationToken({ appId: "org.kde.dolphin" }));
      });
      return btn;
    }, root);

    const btn = h.byTestId("launch");
    h.dispatch(btn, "Click");
    assert.deepEqual(await Promise.all(tokens), ["minted:<omitted>", null]);
    assert.equal(h.activationTokenRequests.length, 1);
    assert.equal(h.activationTokenRequests[0].appId, undefined);
    dispose();
  });
});

test("getCapsLock, useCapsLock, and NativeEvent.modifiers.{capsLock,numLock} isolate per mount region and clean up", async t => {
  await withHost({}, async () => {
    assert.throws(() => useCapsLock(), /useCapsLock requires a Solid owner/);
  });

  const mod = await moduleFor(
    t,
    `
    import {
      LayerSurface,
      PopupSurface,
      SessionLockSurface,
      Show,
      createSignal,
      getCapsLock,
      renderShell,
      requestSessionLock,
      useCapsLock,
    } from "@hewel/zgui";

    export const seenModifiers = [];
    let barCaps = () => false;
    let popupCaps = () => false;
    let lockCaps = () => false;
    let setShowPopup;
    let lockHandle;

    export const readCaps = () => ({
      bar: barCaps(),
      popup: popupCaps(),
      lock: lockCaps(),
    });
    export const hidePopup = () => setShowPopup(false);
    export const unlock = () => lockHandle.unlock();

    function BarContent() {
      barCaps = useCapsLock();
      return (
        <field
          testId="bar-field"
          onKeyDown={ev => {
            seenModifiers.push({
              capsLock: ev.modifiers.capsLock,
              numLock: ev.modifiers.numLock,
            });
          }}
        />
      );
    }

    function PopupContent() {
      popupCaps = useCapsLock();
      return <text>Menu</text>;
    }

    function LockContent() {
      lockCaps = useCapsLock();
      const snap = getCapsLock();
      return <text testId="lock-text">{String(snap)}</text>;
    }

    export function mount() {
      return renderShell(() => {
        const [show, setShow] = createSignal(true);
        setShowPopup = setShow;
        lockHandle = requestSessionLock();
        return (
          <>
            <LayerSurface name="bar" output="DP-1" width={400} height={32}>
              <BarContent />
            </LayerSurface>
            <Show when={show()}>
              <PopupSurface
                name="menu"
                parent="bar"
                width={160}
                height={80}
                anchorRect={{ x: 0, y: 0, width: 20, height: 20 }}
              >
                <PopupContent />
              </PopupSurface>
            </Show>
            <SessionLockSurface name="lock-dp-1" output="DP-1">
              <LockContent />
            </SessionLockSurface>
          </>
        );
      });
    }
    `,
  );

  await withHost(
    {
      monitors: [{ id: "DP-1", name: "DP-1", x: 0, y: 0, width: 1920, height: 1080, scale: 1 }],
      sessionLock: "locked",
      shellSurfaces: true,
    },
    async h => {
      const dispose = h.runTurn(mod.mount);
      h.flush();

      assert.deepEqual(mod.readCaps(), { bar: false, popup: false, lock: false });

      // Update Caps Lock on lock surface only: bar and popup remain false!
      h.emitCapsLock(true, "lock-dp-1");
      assert.deepEqual(mod.readCaps(), { bar: false, popup: false, lock: true });

      // Update Caps Lock on popup, then unmount popup to verify onCleanup unsubscribes cleanly.
      h.emitCapsLock(true, "menu");
      assert.deepEqual(mod.readCaps(), { bar: false, popup: true, lock: true });
      h.runTurn(mod.hidePopup);
      h.flush();

      // Dispatch keydown with explicit capsLock/numLock and default modifiers on bar.
      h.runTurn(mod.unlock);
      h.flush();
      const barField = h.byTestId("bar-field");
      h.dispatch(barField, "KeyDown", { key: "a" });
      h.dispatch(barField, "KeyDown", {
        key: "A",
        modifiers: { capsLock: true, numLock: true },
      });
      assert.deepEqual(mod.seenModifiers, [
        { capsLock: false, numLock: false },
        { capsLock: true, numLock: true },
      ]);

      // Primary region check: disposing renderApp unsubscribes from main bridge.
      const mainRoot = h.makeNode("root");
      let mainCaps = () => false;
      const disposeMain = renderApp(() => {
        mainCaps = useCapsLock();
        assert.equal(getCapsLock(), false);
        return null;
      }, mainRoot);
      assert.equal(mainCaps(), false);
      h.emitCapsLock(true, "main");
      assert.equal(mainCaps(), true);
      disposeMain();
      h.emitCapsLock(false, "main");
      assert.equal(mainCaps(), true, "disposed owner must stop receiving Caps Lock updates");

      dispose();
    },
  );
});
