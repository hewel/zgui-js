import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
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

const source = `
import { Button, Image, Show, createSignal, onCleanup, renderApp, type NativeNode, type DragHandle, type NativeDropEvent } from "@hewel/zgui";
import {
  openAuthorizedDirectory, openSystemDirectory, startFileJob, listTrash, listVolumes,
  loadCachedThumbnail, openExternal, persistSystemGrant, restoreSystemGrant, listGrants,
  openDroppedSystemDirectory, openSystemDirectoryDialog, startOutboundSystemDrag, watchVolumes,
  type ImageHandle, type FileConflictResolution,
} from "@hewel/zgui-services";
export const hostApiVersion = 1;
const code = (error: unknown) => (error as { code?: string })?.code ?? String(error);
export function mount(root: NativeNode) {
  return renderApp(() => {
    const [status, setStatus] = createSignal("idle");
    const [thumbnail, setThumbnail] = createSignal<ImageHandle | null>(null);
    const [watch, setWatch] = createSignal("waiting");
    const [drop, setDrop] = createSignal("idle");
    const [drag, setDrag] = createSignal("idle");
    let image: ImageHandle | null = null;
    let dragNode: NativeNode | undefined;
    let activeDrag: DragHandle | undefined;
    const dir = openSystemDirectory("$HOME/work");
    onCleanup(() => image?.release());
    const disk = openSystemDirectory("/media/disk");
    const workspace = openAuthorizedDirectory("$HOME/work");
    const ref = (relativePath: string) => ({ dir, relativePath });
    watchVolumes(volumes => setWatch(volumes[0]?.mountPoint ?? "none"));
    function denied(operation: () => unknown) {
      try { operation(); } catch (error) { return code(error) === "E_CAPABILITY"; }
      return false;
    }
    async function rejection(operation: () => { promise: Promise<unknown> }) {
      try { await operation().promise; } catch (error) { return code(error); }
      return "unexpected success";
    }
    function check(condition: unknown, label: string) { if (!condition) throw Error(label); }
    async function run(
      external: ReturnType<typeof openExternal>,
      dialog: ReturnType<typeof openSystemDirectoryDialog>,
      secondDialog: ReturnType<typeof openSystemDirectoryDialog>,
    ) {
      try {
        await external.promise;
        check(denied(() => openSystemDirectory("/not-granted")), "ungranted root");
        check(denied(() => openExternal({ url: "https://example.com/", activationToken: "forged" })), "forged activation token");
        check(denied(() => openDroppedSystemDirectory(workspace)), "workspace is not a drop grant");
        const picked = await dialog.promise;
        check(picked?.mode === "read" && picked.name === "disk", "read-only dialog");
        const secondPick = await secondDialog.promise;
        check(secondPick?.name === "work", "second dialog receives the second scripted response");
        secondPick?.close();
        check(denied(() => persistSystemGrant(disk)), "static system directory cannot mint grant");
        const token = await persistSystemGrant(picked!).promise;
        picked?.close();
        await workspace.stat("external/photo.png").promise.then(() => { throw Error("workspace escape"); }, err => check(code(err) === "E_PATH_ESCAPE", "workspace error"));
        const broken = await dir.lstat("broken").promise;
        const link = await dir.lstat("external").promise;
        check(broken.kind === "symlink" && broken.symlinkBroken && broken.symlinkTarget === "missing", "broken");
        check(!link.symlinkBroken && link.symlinkTargetKind === "dir", "external link metadata");
        check((await dir.readDir("external").promise).entries[0]?.name === "photo.png", "active cross-root grant");
        const first = await dir.readDir("many", { pageSize: 5000 }).promise;
        const second = await dir.readDir("many", { pageSize: 5000, cursor: first.nextCursor! }).promise;
        const last = await dir.readDir("many", { pageSize: 5000, cursor: second.nextCursor! }).promise;
        check(first.totalEntries === 10005 && first.entries[1]?.name === "img2.png" && second.entries[0]?.name === "img5001.png" && last.entries.length === 5 && last.nextCursor === null, "pagination");
        const beforeCreate = rejection(() => dir.lstat("ordered"));
        const create = startFileJob({ operation: "mkdir", entry: ref("ordered") }).promise;
        const afterCreate = dir.lstat("ordered").promise;
        const remove = startFileJob({ operation: "removePermanent", entries: [ref("ordered")] }).promise;
        const afterRemove = rejection(() => dir.lstat("ordered"));
        const [before, , created, , after] = await Promise.all([beforeCreate, create, afterCreate, remove, afterRemove]);
        check(before === "NotFound" && created.kind === "dir" && after === "NotFound", "reads and jobs run in call order");
        const progress: string[] = [];
        await startFileJob({ operation: "copy", sources: [ref("a.txt"), ref("stale.png"), ref("broken")], destination: ref("dst") }, {
          onConflict: async () => {
            check((await dir.lstat("dst/a.txt").promise).size === 4, "conflict handler can inspect the paused destination");
            return { action: "autoRename" };
          },
          onProgress: p => { progress.push([p.itemsCompleted, p.itemsTotal, p.currentPath, p.bytesCompleted].join("/")); },
        }).promise;
        check(progress.join(",") === "1/3/a.txt/3,2/3/stale.png/10,3/3/broken/10", "deterministic copy progress: " + progress.join(","));
        check((await dir.lstat("dst/a (1).txt").promise).size === 3, "copy renamed conflict");
        await startFileJob({ operation: "move", sources: [ref("a.txt")], destination: ref("dst") }, {
          onConflict: () => ({ action: "overwrite" }),
        }).promise;
        await dir.lstat("a.txt").promise.then(() => { throw Error("move retained source"); }, err => check(code(err) === "NotFound", "move error"));
        await startFileJob({ operation: "trash", entries: [ref("dst/a.txt")] }).promise;
        const items = await listTrash().promise;
        check(items.length === 1 && items[0]?.name === "a.txt", "trash inventory");
        await startFileJob({ operation: "copy", sources: [ref("dst/a (1).txt")], destination: ref("") }).promise;
        await startFileJob({ operation: "createSymlink", entry: ref("dst/a.txt"), target: "a (1).txt" }).promise;
        await startFileJob({ operation: "restoreFromTrash", ids: [items[0]!.id] }, { onConflict: () => ({ action: "skip" }) }).promise;
        check((await listTrash().promise).length === 1, "skip retains Trash");
        await startFileJob({ operation: "restoreFromTrash", ids: [items[0]!.id] }, { onConflict: () => ({ action: "overwrite" }) }).promise;
        check((await dir.lstat("dst/a.txt").promise).kind === "file" && (await listTrash().promise).length === 0, "restore");
        await startFileJob({ operation: "setPermissions", entry: ref("dst/a.txt"), mode: 0o100640 }).promise;
        check((await dir.lstat("dst/a.txt").promise).mode === 0o640, "chmod");
        check((await listGrants().promise).some(grant => grant.token === token && grant.kind === "system-dir"), "shared grant ledger");
        const restored = await restoreSystemGrant(token).promise;
        check((await restored.lstat("photo.png").promise).kind === "file", "restore grant");
        restored.close();
        check((await listVolumes().promise)[0]?.mountPoint === "/media/disk", "volumes");
        const hit = await loadCachedThumbnail({ dir: disk, relativePath: "photo.png" }).promise;
        const stale = await loadCachedThumbnail({ ...ref("stale.png"), size: "large" }).promise;
        check(hit?.src.startsWith("zgui-blob:") && stale === null, "thumbnail freshness");
        image = hit;
        setThumbnail(hit);
        setStatus("ready");
      } catch (error) { setStatus(code(error)); }
    }
    let cancelPaused: (() => void) | undefined;
    function pause() {
      const job = startFileJob({ operation: "copy", sources: [ref("a (1).txt")], destination: ref("dst") }, {
        onConflict: () => {
          setStatus("paused");
          return new Promise<FileConflictResolution>(() => {});
        },
      });
      cancelPaused = () => job.cancel();
      // This must stay queued until the conflict is answered or cancelled.
      const queued = startFileJob({ operation: "mkdir", entry: ref("queued-after-conflict") });
      void queued.promise.catch(error => setStatus(code(error)));
      void job.promise.catch(error => setStatus(code(error)));
    }
    async function receiveDrop(event: NativeDropEvent) {
      try {
        const directory = event.directories[0];
        const [folder, file, covered, link] = event.systemEntries;
        check(directory && folder && file && covered && link && event.systemEntries.length === 4, "directory, file and symlink entries");
        const legacyRoot = openDroppedSystemDirectory(directory!);
        try {
          check((await legacyRoot.readDir().promise).entries[0]?.name === "dropped.md", "legacy directory redemption");
        } finally { legacyRoot.close(); }
        check(folder!.dir.mode === "read" && folder!.relativePath === "", "dropped directory is read-only");
        const page = await folder!.dir.readDir().promise;
        check(page.entries[0]?.name === "dropped.md", "dropped directory listing");
        await persistSystemGrant(folder!.dir).promise;
        check(file!.dir.mode === "read" && file!.relativePath === "dropped.png", "single file grant");
        check((await file!.dir.lstat(file!.relativePath).promise).kind === "file", "single file stat");
        check(await rejection(() => file!.dir.lstat("sibling.png")) === "E_CAPABILITY", "sibling stat denied");
        check(await rejection(() => file!.dir.readDir()) === "E_CAPABILITY", "parent readDir denied");
        check(await rejection(() => file!.dir.readDir(file!.relativePath)) === "E_CAPABILITY", "entry readDir denied");
        check(denied(() => persistSystemGrant(file!.dir)), "file drop cannot grant parent");
        const cached = await loadCachedThumbnail(file!).promise;
        check(cached?.src.startsWith("zgui-blob:"), "single file thumbnail");
        cached?.release();
        check(await rejection(() => loadCachedThumbnail({ dir: file!.dir, relativePath: "sibling.png" })) === "E_CAPABILITY", "sibling thumbnail denied");
        check((await link!.dir.lstat(link!.relativePath).promise).kind === "symlink", "inspect dropped symlink");
        check(await rejection(() => loadCachedThumbnail(link!)) === "E_CAPABILITY", "dropped symlink target denied");
        await startFileJob({ operation: "copy", sources: [file!, link!], destination: ref("dst") }).promise;
        check((await dir.lstat("dst/dropped.png").promise).size === 7, "single file copy");
        check((await dir.lstat("dst/link").promise).kind === "symlink", "copy link not target");
        check(await rejection(() => startFileJob({ operation: "copy", sources: [{ dir: file!.dir, relativePath: "sibling.png" }], destination: ref("dst") })) === "E_CAPABILITY", "sibling copy denied");
        check(covered!.dir.mode === "readwrite" && covered!.relativePath === "to-move.txt", "covered file preserves active write authority");
        await startFileJob({ operation: "move", sources: [covered!], destination: ref("dst") }).promise;
        setDrop((await dir.lstat("dst/to-move.txt").promise).kind === "file" ? "drop passed" : "move failed");
      } catch (error) {
        setDrop(code(error));
      } finally {
        for (const entry of event.systemEntries) entry.dir.close();
        for (const handle of [...event.files, ...event.directories]) handle.close();
      }
    }
    return <column>
      <Button testId="run" onClick={() => { void run(
        openExternal(ref("external/photo.png")),
        openSystemDirectoryDialog({ readOnly: true }),
        openSystemDirectoryDialog(),
      ); }}>Run</Button>
      <Button testId="pause" onClick={pause}>Pause</Button>
      <Button testId="cancel-paused" onClick={() => cancelPaused?.()}>Cancel paused job</Button>
      <text testId="status">{status()}</text>
      <box testId="drop-zone" style="width: 100px; height: 24px"
        onDragOver={event => event.accept("copy")}
        onDrop={event => void receiveDrop(event)}
      >Drop</box>
      <box testId="drag-source" ref={node => { dragNode = node; }} style="width: 100px; height: 24px"
        onPointerDown={() => {
          if (!dragNode) return;
          const node = dragNode;
          const entries = [{ dir: disk, relativePath: "photo.png" }];
          if (denied(() => startOutboundSystemDrag(node, {entries, allowedEffects: ["move"]}))) {
            activeDrag = startOutboundSystemDrag(node, {entries, onEnd: outcome => setDrag(outcome.reason)});
          }
        }}
        onPointerUp={() => activeDrag?.cancel()}
      >Drag</box>
      <text testId="watch">{watch()}</text>
      <text testId="drop-log">{drop()}</text>
      <text testId="drag-log">{drag()}</text>
      <Show when={thumbnail()} keyed>
        {handle => <Image testId="cached-image" src={handle.src} style="width: 24px; height: 24px" />}
      </Show>
    </column>;
  }, root);
}
`;

test("preview system files preserve workspace confinement and retire a job paused on conflict", async t => {
  const temp = await createTempWorkspace("preview-system-files-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "system-files-preview", source, {
    rawSource: true,
    capabilities: {
      files: { directories: [{ path: "$HOME/work", mode: "read" }] },
      systemFiles: {
        read: true,
        write: true,
        trash: true,
        volumes: true,
        thumbnails: true,
        openExternal: true,
        directories: [
          { path: "$HOME/work", mode: "readwrite" },
          { path: "/media/disk", mode: "read" },
        ],
      },
    },
  });
  const fixture = path.join(temp.dir, "fixture.json");
  const work = "/home/preview/work";
  const png =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAAAAAA6fptVAAAACklEQVR4nGNgAAAAAgABSK+kcQAAAABJRU5ErkJggg==";
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "system-files",
    events: [],
    services: {
      files: {
        dirs: [work, "/media/disk", "/Incoming"],
        files: {
          "/media/disk/photo.png": "original",
          "/Elsewhere/dropped.png": "dropped",
          [`${work}/to-move.txt`]: "move",
        },
        symlinks: { [`${work}/external`]: "/media/disk" },
      },
      systemFiles: {
        dirs: [`${work}/dst`, `${work}/many`, "/media/disk"],
        files: {
          [`${work}/a.txt`]: "new",
          [`${work}/dst/a.txt`]: "old!",
          [`${work}/stale.png`]: "changed",
          "/media/disk/photo.png": "original",
          "/Incoming/dropped.md": "dropped",
          "/Elsewhere/dropped.png": "dropped",
          "/Elsewhere/sibling.png": "hidden",
          [`${work}/to-move.txt`]: "move",
        },
        symlinks: {
          [`${work}/broken`]: "missing",
          [`${work}/external`]: "/media/disk",
          "/Elsewhere/link": "sibling.png",
        },
        generated: [{ directory: `${work}/many`, prefix: "img", suffix: ".png", count: 10005 }],
        volumes: [
          {
            mountPoint: "/media/disk",
            source: "/dev/sdz1",
            fileSystem: "ext4",
            readOnly: false,
            removable: true,
          },
        ],
        dialogs: ["/media/disk", work],
        thumbnails: [
          { path: "/media/disk/photo.png", mtime: 0, png },
          { path: "/Elsewhere/dropped.png", mtime: 0, png },
          { path: `${work}/stale.png`, size: "large", mtime: 1, png },
        ],
      },
    },
  });
  const click = (id, testId) => ({
    id,
    op: "pointer.click",
    button: "primary",
    target: { surface: "main", testId },
  });
  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "system-files",
    requiredCapabilities: [],
    steps: [
      click("run", "run"),
      {
        id: "external-call",
        op: "assert",
        serviceCall: { service: "system-files", method: "openExternal", count: 1 },
      },
      {
        id: "drop",
        op: "drag.drop",
        target: { surface: "main", testId: "drop-zone" },
        paths: ["/Incoming", "/Elsewhere/dropped.png", `${work}/to-move.txt`, "/Elsewhere/link"],
      },
      {
        id: "drag-down",
        op: "pointer.down",
        button: "primary",
        target: { surface: "main", testId: "drag-source" },
      },
      {
        id: "drag-up",
        op: "pointer.up",
        button: "primary",
        target: { surface: "main", testId: "drag-source" },
      },
      { id: "ready", op: "capture", captureId: "ready", surface: "main" },
      click("pause", "pause"),
      { id: "paused", op: "capture", captureId: "paused", surface: "main" },
      click("cancel-paused", "cancel-paused"),
      { id: "cancelled", op: "capture", captureId: "cancelled", surface: "main" },
      click("pause-again", "pause"),
      { id: "retire-paused-job", op: "surface.recreate", surface: "main" },
    ],
  });
  const ready = await readTree(out, "ready");
  assert.equal(getNode(ready, "status")?.text, "ready");
  assert.equal(getNode(ready, "watch")?.text, "/media/disk");
  assert.equal(getNode(ready, "drop-log")?.text, "drop passed");
  assert.equal(getNode(ready, "drag-log")?.text, "cancel");
  assert.ok(getNode(ready, "cached-image"));
  assert.equal(getNode(await readTree(out, "paused"), "status")?.text, "paused");
  assert.equal(getNode(await readTree(out, "cancelled"), "status")?.text, "Cancelled");
  const steps = (await readFile(path.join(out, "steps.jsonl"), "utf8"))
    .trim()
    .split("\n")
    .map(line => JSON.parse(line));
  const retired = steps.find(step => step.id === "retire-paused-job")?.result.retired;
  assert.equal(retired?.before.fileJobs, 2);
  assert.equal(retired?.before.fileConflicts, 1);
  assert.equal(retired?.clean, true);
  const { records } = JSON.parse(await readFile(path.join(out, "services.json"), "utf8"));
  const external = records.filter(record => record.kind === "externalOpen");
  assert.deepEqual(
    external.map(record => record.detail.target),
    ["file:///media/disk/photo.png"],
  );
  assert.equal(result.teardown.clean, true);
});

test("preview rejects undeclared sub-capabilities and cannot restore a grant above the write ceiling", async t => {
  const temp = await createTempWorkspace("preview-system-files-read-only-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "system-files-read-only",
    `
    import { renderApp, createSignal, type NativeNode } from "@hewel/zgui";
    import { openSystemDirectory, restoreSystemGrant, watchVolumes, listVolumes, listTrash, openExternal, loadCachedThumbnail, startFileJob } from "@hewel/zgui-services";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => {
        const [state, setState] = createSignal("restoring");
        const dir = openSystemDirectory("/chosen");
        const entry = { dir, relativePath: "file" };
        let count = 0;
        for (const attempt of [
          () => openSystemDirectory("/write-root"),
          () => startFileJob({ operation: "mkdir", entry }),
          () => listVolumes(),
          () => watchVolumes(() => {}),
          () => listTrash(),
          () => loadCachedThumbnail(entry),
          () => openExternal({ url: "https://example.com/" }),
        ]) {
          try { attempt(); } catch (error) {
            if (error instanceof Error && "code" in error && error.code === "E_CAPABILITY") count++;
          }
        }
        void restoreSystemGrant("writable").promise.then(
          handle => { handle.close(); setState("unexpected restore"); },
          error => setState(String(count) + ":" + String(error.code)),
        );
        return <text testId="result">{state()}</text>;
      }, root);
    }
    `,
    {
      rawSource: true,
      capabilities: {
        systemFiles: {
          read: true,
          directories: [
            { path: "/chosen", mode: "read" },
            { path: "/write-root", mode: "readwrite" },
          ],
        },
      },
    },
  );
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "read-only-system-files",
    events: [],
    services: {
      files: {
        grants: [
          {
            token: "writable",
            path: "/chosen",
            kind: "system-dir",
            mode: "readwrite",
            label: "Chosen",
            createdMs: 0,
          },
        ],
      },
      systemFiles: { files: { "/chosen/file": "keep", "/write-root/file": "keep" } },
    },
  });
  const { out } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "system-files-read-only",
    requiredCapabilities: [],
    steps: [{ id: "result", op: "capture", captureId: "result", surface: "main" }],
  });
  assert.equal(getNode(await readTree(out, "result"), "result")?.text, "7:E_CAPABILITY");
});

test("preview without a usable Trash leaves the original entry in place", async t => {
  const temp = await createTempWorkspace("preview-system-files-no-trash-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "no-trash",
    `
    import { renderApp, createSignal, type NativeNode } from "@hewel/zgui";
    import { openSystemDirectory, startFileJob } from "@hewel/zgui-services";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => {
        const [state, setState] = createSignal("starting");
        const dir = openSystemDirectory("/chosen");
        void startFileJob({ operation: "trash", entries: [{ dir, relativePath: "report" }] }).promise.then(
          () => setState("unexpected success"),
          async error => {
            const entry = await dir.lstat("report").promise;
            setState(String(error.code) + ":" + String(entry.size));
          },
        );
        return <text testId="result">{state()}</text>;
      }, root);
    }
    `,
    {
      rawSource: true,
      capabilities: {
        systemFiles: {
          read: true,
          write: true,
          trash: true,
          directories: [{ path: "/chosen", mode: "readwrite" }],
        },
      },
    },
  );
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "no-trash",
    events: [],
    services: { systemFiles: { files: { "/chosen/report": "keep" }, trashUnavailable: true } },
  });
  const { out } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "no-trash",
    requiredCapabilities: [],
    steps: [{ id: "result", op: "capture", captureId: "result", surface: "main" }],
  });
  assert.equal(getNode(await readTree(out, "result"), "result")?.text, "E_TRASH_UNAVAILABLE:4");
});
