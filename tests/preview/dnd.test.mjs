import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  readTree,
  runScenario,
  writeJson,
} from "./harness.mjs";

const dndAppSource = `
import {
  Button,
  For,
  TextField,
  createSignal,
  focus,
  onCleanup,
  renderApp,
  scrollTo,
  startInternalDrag,
} from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [items, setItems] = createSignal(["Alpha", "Beta", "Gamma"]);
    const [endLog, setEndLog] = createSignal("NONE");
    const [hoverLog, setHoverLog] = createSignal("NONE");
    const [dropLog, setDropLog] = createSignal("NONE");
    const [focusedDropLog, setFocusedDropLog] = createSignal("CLEAN");
    const [rejectedDropLog, setRejectedDropLog] = createSignal("CLEAN");

    let scrollerEl: NativeNode | undefined;
    let focusInputEl: NativeNode | undefined;
    const openHandles: { close(): void }[] = [];
    onCleanup(() => {
      for (const h of openHandles) h.close();
    });

    const reorder = (fromItem: string, toItem: string) => {
      if (fromItem === toItem) return;
      setItems(prev => {
        const next = prev.filter(x => x !== fromItem);
        const idx = next.indexOf(toItem);
        if (idx === -1) return prev;
        next.splice(idx + 1, 0, fromItem);
        return next;
      });
    };

    return (
      <column style="width: 100%; height: 100%; padding: 8px; gap: 6px">
        <row style="gap: 8px">
          <TextField
            testId="other-focus"
            ref={el => {
              focusInputEl = el;
            }}
            value="focused"
            onDrop={() => {
              setFocusedDropLog("WRONG_FOCUSED_TARGET");
            }}
          />
          <Button
            testId="btn-prep"
            onClick={() => {
              if (scrollerEl) scrollTo(scrollerEl, { top: 200, behavior: "instant" });
              if (focusInputEl) focus(focusInputEl);
            }}
          >
            Prep Scroll + Focus
          </Button>
        </row>

        <row style="gap: 8px">
          <column style="width: 160px; gap: 4px">
            <For each={items()}>
              {item => {
                let rowNode: NativeNode | undefined;
                return (
                  <box
                    testId={\`row-\${item}\`}
                    ref={el => {
                      rowNode = el;
                    }}
                    style="height: 28px; padding: 4px; background-color: #262626"
                    onPointerDown={e => {
                      if (!rowNode) return;
                      // Pointer capture remains on source while chain_at hit-tests drop target.
                      e.capturePointer();
                      startInternalDrag(rowNode, {
                        types: ["application/x-zgui-row"],
                        payload: { item },
                        onEnd(res) {
                          setEndLog(\`dropped=\${String(res.dropped)},effect=\${res.effect},reason=\${res.reason}\`);
                        },
                      });
                    }}
                    onPointerUp={e => e.releasePointer()}
                    onPointerCancel={e => e.releasePointer()}
                    onDragOver={e => {
                      if (e.source === "internal" && e.types.includes("application/x-zgui-row")) {
                        e.accept("move");
                      }
                    }}
                    onDrop={e => {
                      const data = e.payload as { item?: string } | undefined;
                      if (e.source === "internal" && data?.item) {
                        reorder(data.item, item);
                      }
                    }}
                  >
                    <text>{item}</text>
                  </box>
                );
              }}
            </For>
          </column>

          <column
            testId="scroller"
            ref={el => {
              scrollerEl = el;
            }}
            style="width: 220px; height: 110px; overflow-y: scroll; gap: 6px; background-color: #181818"
          >
            <box style="height: 180px; flex-shrink: 0; background-color: #222222">
              <text>Spacer</text>
            </box>
            <box
              testId="scrolled-drop-zone"
              style="height: 48px; flex-shrink: 0; background-color: #1f3328; padding: 6px"
              onDragEnter={e => {
                const leakedKeys = "files" in e || "paths" in e || "directories" in e;
                setHoverLog(
                  \`enter:\${e.source}:\${e.types.join(",")}:n=\${e.fileCount}:leak=\${String(leakedKeys)}\`,
                );
                if (e.types.includes("inode/directory")) e.accept("move");
              }}
              onDragOver={e => {
                if (e.source === "external" && e.types.includes("text/uri-list") && !e.types.includes("inode/directory")) {
                  e.accept("copy");
                }
              }}
              onDrop={e => {
                const dir = e.directories[0];
                if (dir) {
                  openHandles.push(dir, ...e.files);
                  const escape = dir.readText("outside").promise.then(() => "UNEXPECTED", error => error.code);
                  void Promise.all([dir.readDir().promise, dir.stat().promise, escape]).then(([entries, stat, escaped]) => {
                    setDropLog(\`effect=\${e.effect},files=\${e.files.length},name=\${dir.name},mode=\${dir.mode},dir=\${stat.isDir},entries=\${entries.map(entry => entry.name).join(",")},escape=\${escaped}\`);
                  });
                  return;
                }
                if (e.source !== "external" || e.files.length === 0) return;
                const file = e.files[0]!;
                openHandles.push(file);
                const req = file.readText();
                void req.promise.then(text => {
                  setDropLog(\`name=\${file.name},mode=\${file.mode},text=\${text}\`);
                });
              }}
            >
              <text>Drop Zone</text>
            </box>
            <box
              testId="reject-drop-zone"
              style="height: 36px; flex-shrink: 0; background-color: #332020; padding: 6px"
              onDragOver={() => {
                // Intentionally does NOT call e.accept().
              }}
              onDrop={() => {
                setRejectedDropLog("UNEXPECTED_DROP");
              }}
            >
              <text>Reject Zone</text>
            </box>
          </column>
        </row>

        <text testId="order-log">{items().join(",")}</text>
        <text testId="end-log">{endLog()}</text>
        <text testId="hover-log">{hoverLog()}</text>
        <text testId="drop-log">{dropLog()}</text>
        <text testId="focused-drop-log">{focusedDropLog()}</text>
        <text testId="rejected-drop-log">{rejectedDropLog()}</text>
      </column>
    );
  }, root);
}
`;

test("preview DnD supports internal hit-tested reorder with source pointer capture, Escape cancel, and external file drop onto an unfocused scrolled target (#66)", async t => {
  const temp = await createTempWorkspace("preview-dnd-");
  t.after(() => temp.cleanup());

  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "dnd-preview", dndAppSource, {
    rawSource: true,
  });

  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "dnd-preview",
    services: {
      files: {
        dirs: ["/home/preview/Incoming", "/home/preview/Incoming/Album"],
        files: {
          "/home/preview/Incoming/dropped.md": "hello from external drop",
          "/home/preview/Incoming/Album/photo.txt": "inside album",
        },
        symlinks: {
          "/home/preview/Incoming/Album/outside": "/home/preview/Incoming/dropped.md",
        },
      },
    },
    events: [],
  });

  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "dnd-preview",
    requiredCapabilities: [],
    steps: [
      // 1. Internal reorder: drag Alpha onto Gamma while Alpha holds pointer capture.
      {
        id: "drag-start-alpha",
        op: "pointer.down",
        button: "primary",
        target: { surface: "main", testId: "row-Alpha" },
      },
      {
        id: "drag-over-gamma",
        op: "pointer.move",
        target: { surface: "main", testId: "row-Gamma" },
      },
      {
        id: "drag-drop-gamma",
        op: "pointer.up",
        button: "primary",
        target: { surface: "main", testId: "row-Gamma" },
      },
      { id: "settle-reorder", op: "clock.advance", byMs: 16 },
      { id: "cap-reorder", op: "capture", captureId: "reorder", surface: "main" },

      // 2. Escape cancels an active internal drag cleanly.
      {
        id: "drag-start-beta",
        op: "pointer.down",
        button: "primary",
        target: { surface: "main", testId: "row-Beta" },
      },
      { id: "drag-esc", op: "key.press", surface: "main", key: "Escape" },
      {
        id: "drag-up-after-esc",
        op: "pointer.up",
        button: "primary",
        target: { surface: "main", testId: "row-Beta" },
      },
      { id: "settle-esc", op: "clock.advance", byMs: 16 },
      { id: "cap-esc", op: "capture", captureId: "esc", surface: "main" },

      // 3. Scroll drop zone into view while keeping keyboard focus on `other-focus`,
      //    then drop an external file onto the unfocused scrolled target!
      {
        id: "prep-scroll-and-focus",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "btn-prep" },
      },
      { id: "settle-scroll", op: "clock.advance", byMs: 16 },
      // First verify non-accepting target does NOT receive onDrop or mint handles.
      {
        id: "ext-drop-reject",
        op: "drag.drop",
        target: { surface: "main", testId: "reject-drop-zone" },
        paths: ["/home/preview/Incoming/dropped.md"],
      },
      // Now hover + drop onto the unfocused scrolled drop zone.
      {
        id: "ext-enter",
        op: "drag.enter",
        target: { surface: "main", testId: "scrolled-drop-zone" },
        paths: ["/home/preview/Incoming/dropped.md"],
      },
      {
        id: "ext-move",
        op: "drag.move",
        target: { surface: "main", testId: "scrolled-drop-zone" },
      },
      {
        id: "ext-drop",
        op: "drag.drop",
        target: { surface: "main", testId: "scrolled-drop-zone" },
        paths: ["/home/preview/Incoming/dropped.md"],
      },
      { id: "settle-drop", op: "clock.advance", byMs: 32 },
      { id: "cap-done", op: "capture", captureId: "done", surface: "main" },
      {
        id: "ext-folder-enter",
        op: "drag.enter",
        target: { surface: "main", testId: "scrolled-drop-zone" },
        paths: ["/home/preview/Incoming/Album", "/home/preview/Incoming/dropped.md"],
      },
      {
        id: "ext-folder-mismatch",
        op: "drag.drop",
        action: "copy",
        target: { surface: "main", testId: "scrolled-drop-zone" },
        paths: ["/home/preview/Incoming/Album", "/home/preview/Incoming/dropped.md"],
      },
      { id: "cap-mismatch", op: "capture", captureId: "mismatch", surface: "main" },
      {
        id: "ext-folder-enter-again",
        op: "drag.enter",
        target: { surface: "main", testId: "scrolled-drop-zone" },
        paths: ["/home/preview/Incoming/Album", "/home/preview/Incoming/dropped.md"],
      },
      {
        id: "ext-folder-hover",
        op: "drag.move",
        target: { surface: "main", testId: "scrolled-drop-zone" },
      },
      {
        id: "ext-folder-move",
        op: "drag.drop",
        action: "move",
        target: { surface: "main", testId: "scrolled-drop-zone" },
        paths: ["/home/preview/Incoming/Album", "/home/preview/Incoming/dropped.md"],
      },
      { id: "settle-folder", op: "clock.advance", byMs: 32 },
      { id: "cap-folder", op: "capture", captureId: "folder", surface: "main" },
    ],
  });

  assert.equal(result.teardown.clean, true, JSON.stringify(result.teardown));

  const reorderTree = await readTree(out, "reorder");
  assert.equal(getNode(reorderTree, "order-log")?.text, "Beta,Gamma,Alpha");
  assert.equal(getNode(reorderTree, "end-log")?.text, "dropped=true,effect=move,reason=drop");

  const escTree = await readTree(out, "esc");
  assert.equal(getNode(escTree, "end-log")?.text, "dropped=false,effect=none,reason=escape");

  const doneTree = await readTree(out, "done");
  assert.equal(
    getNode(doneTree, "hover-log")?.text,
    "enter:external:text/uri-list,text/markdown:n=1:leak=false",
  );
  assert.equal(
    getNode(doneTree, "drop-log")?.text,
    "name=dropped.md,mode=read,text=hello from external drop",
  );
  assert.equal(getNode(doneTree, "focused-drop-log")?.text, "CLEAN");
  assert.equal(getNode(doneTree, "rejected-drop-log")?.text, "CLEAN");
  const mismatchTree = await readTree(out, "mismatch");
  assert.equal(getNode(mismatchTree, "drop-log")?.text, getNode(doneTree, "drop-log")?.text);
  const folderTree = await readTree(out, "folder");
  assert.equal(
    getNode(folderTree, "hover-log")?.text,
    "enter:external:text/uri-list,inode/directory,text/markdown:n=2:leak=false",
  );
  assert.equal(
    getNode(folderTree, "drop-log")?.text,
    "effect=move,files=1,name=Album,mode=read,dir=true,entries=photo.txt,escape=E_PATH_ESCAPE",
  );
});

const outboundDragSource = `
import { createSignal, renderApp, startOutboundDrag } from "@hewel/zgui";
import type { DragHandle, NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [beforePress, setBeforePress] = createSignal("PENDING");
    const [duringPress, setDuringPress] = createSignal("PENDING");
    const [endResult, setEndResult] = createSignal("PENDING");
    const [afterRelease, setAfterRelease] = createSignal("PENDING");

    let dragNode: NativeNode | undefined;
    let active: DragHandle | undefined;

    return (
      <column style="width: 100%; height: 100%; padding: 12px; gap: 8px">
        <box
          testId="btn-before"
          style="height: 28px; padding: 4px; background-color: #262626"
          onClick={() => {
            if (!dragNode) return;
            // Inside click (after pointer release) -> no live press active!
            try {
              startOutboundDrag(dragNode, {
                items: [{ mime: "text/plain", data: "no-press" }],
              });
              setBeforePress("UNEXPECTED_ALLOW");
            } catch (err) {
              const code = err instanceof Error && "code" in err ? String(err.code) : "ERR";
              setBeforePress(\`DENIED:\${code}\`);
            }
          }}
        >
          <text>Check Before Press</text>
        </box>
        <box
          testId="drag-source"
          ref={el => {
            dragNode = el;
          }}
          style="height: 36px; padding: 6px; background-color: #243b55"
          onPointerDown={() => {
            if (!dragNode) return;
            active = startOutboundDrag(dragNode, {
              items: [{ mime: "text/plain", data: "copied-out" }],
              allowedEffects: ["copy", "move"],
              onEnd(res) {
                setEndResult(\`accepted=\${String(res.accepted)},effect=\${res.effect},reason=\${res.reason}\`);
              },
            });
            setDuringPress(\`STARTED:\${active.id}\`);
          }}
          onPointerUp={() => {
            active?.cancel();
            active = undefined;
            if (!dragNode) return;
            try {
              startOutboundDrag(dragNode, {
                items: [{ mime: "text/plain", data: "after-release" }],
              });
              setAfterRelease("UNEXPECTED_ALLOW");
            } catch (err) {
              const code = err instanceof Error && "code" in err ? String(err.code) : "ERR";
              setAfterRelease(\`DENIED:\${code}\`);
            }
          }}
        >
          <text>Outbound Source</text>
        </box>
        <text testId="before-press">{beforePress()}</text>
        <text testId="during-press">{duringPress()}</text>
        <text testId="end-result">{endResult()}</text>
        <text testId="after-release">{afterRelease()}</text>
      </column>
    );
  }, root);
}
`;

test("preview outbound Copy-only drag enforces live press serial, cancels cleanly, and tears down cleanly (#67)", async t => {
  const temp = await createTempWorkspace("preview-outbound-dnd-");
  t.after(() => temp.cleanup());

  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "outbound-dnd",
    outboundDragSource,
    { rawSource: true },
  );

  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "outbound-dnd",
    requiredCapabilities: [],
    steps: [
      {
        id: "click-before",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "btn-before" },
      },
      {
        id: "press-start",
        op: "pointer.down",
        button: "primary",
        target: { surface: "main", testId: "drag-source" },
      },
      {
        id: "release-cancel",
        op: "pointer.up",
        button: "primary",
        target: { surface: "main", testId: "drag-source" },
      },
      { id: "settle", op: "clock.advance", byMs: 16 },
      { id: "cap-done", op: "capture", captureId: "done", surface: "main" },
    ],
  });

  assert.equal(result.teardown.clean, true, JSON.stringify(result.teardown));

  const tree = await readTree(out, "done");
  assert.equal(getNode(tree, "before-press")?.text, "DENIED:E_DRAG_NO_LIVE_PRESS");
  assert.match(getNode(tree, "during-press")?.text ?? "", /^STARTED:\d+$/);
  assert.equal(getNode(tree, "end-result")?.text, "accepted=false,effect=none,reason=cancel");
  assert.equal(getNode(tree, "after-release")?.text, "DENIED:E_DRAG_NO_LIVE_PRESS");
});
