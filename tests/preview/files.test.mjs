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

const filesPreviewSource = `
import { Button, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  listBookmarks,
  openAuthorizedDirectory,
  openDirectoryDialog,
  openFileDialog,
  persistHandle,
  restoreBookmark,
  revokeBookmark,
  saveFileDialog,
  watchDirectory,
} from "@hewel/zgui-services";
import type { DirHandle, FileHandle } from "@hewel/zgui-services";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [dialogBgDenied, setDialogBgDenied] = createSignal("PENDING");
    const [unauthDirDenied, setUnauthDirDenied] = createSignal("PENDING");
    const [escapeStatus, setEscapeStatus] = createSignal("PENDING");
    const [dialogStatus, setDialogStatus] = createSignal("PENDING");
    const [bookmarkStatus, setBookmarkStatus] = createSignal("PENDING");
    const [watchLog, setWatchLog] = createSignal("NONE");

    const openHandles: (FileHandle | DirHandle)[] = [];
    onCleanup(() => {
      for (const h of openHandles) h.close();
    });

    // 1. Dialog outside user gesture must throw E_CAPABILITY.
    try {
      openFileDialog().cancel();
      setDialogBgDenied("UNEXPECTED_ALLOW");
    } catch (err) {
      const code = err instanceof Error && "code" in err ? String(err.code) : "UNKNOWN";
      setDialogBgDenied(\`DENIED:\${code}\`);
    }

    // 2. Undeclared directory must throw E_CAPABILITY.
    try {
      const bad = openAuthorizedDirectory("/etc");
      bad.close();
      setUnauthDirDenied("UNEXPECTED_ALLOW");
    } catch (err) {
      const code = err instanceof Error && "code" in err ? String(err.code) : "UNKNOWN";
      setUnauthDirDenied(\`DENIED:\${code}\`);
    }

    // 3. Authorized manifest directory: read, atomic write, mkdir/rename/remove, symlink-escape rejection, watchDirectory.
    const notesDir = openAuthorizedDirectory("$XDG_DOCUMENTS_DIR/notes");
    watchDirectory(notesDir, { recursive: true, debounceMs: 50 }, events => {
      const summary = events.map(e => \`\${e.kind}:\${e.path}\`).join(",");
      setWatchLog(prev => (prev === "NONE" ? summary : \`\${prev}|\${summary}\`));
    });

    void (async () => {
      const initial = await notesDir.readText("welcome.md").promise;
      await notesDir.mkdir("sub", { recursive: true }).promise;
      await notesDir.writeText("sub/draft.md", "hello-draft").promise;
      await notesDir.rename("sub/draft.md", "sub/final.md").promise;
      const finalText = await notesDir.readText("sub/final.md").promise;
      const entries = await notesDir.readDir().promise;

      let symlinkCode = "NONE";
      try {
        await notesDir.readText("escape-link").promise;
      } catch (err) {
        symlinkCode = err instanceof Error && "code" in err ? String(err.code) : "ERR";
      }

      let traversalCode = "NONE";
      try {
        await notesDir.readText("../secret.txt").promise;
      } catch (err) {
        traversalCode = err instanceof Error && "code" in err ? String(err.code) : "ERR";
      }

      setEscapeStatus(
        \`init=\${initial},final=\${finalText},entries=\${entries.map(e => e.name).join(",")},sym=\${symlinkCode},trav=\${traversalCode}\`,
      );
    })();

    return (
      <column style="width: 100%; height: 100%; padding: 12px; gap: 6px">
        <text testId="dialog-bg">{dialogBgDenied()}</text>
        <text testId="unauth-dir">{unauthDirDenied()}</text>
        <text testId="escape-status">{escapeStatus()}</text>
        <text testId="dialog-status">{dialogStatus()}</text>
        <text testId="bookmark-status">{bookmarkStatus()}</text>
        <text testId="watch-log">{watchLog()}</text>
        <row style="gap: 6px">
          <Button
            testId="btn-dialogs"
            onClick={() => {
              const openReq = openFileDialog({ title: "Open Note" });
              const saveReq = saveFileDialog({ suggestedName: "saved.md" });
              const dirReq = openDirectoryDialog({ title: "Open Workspace" });
              void (async () => {
                const file = await openReq.promise;
                const saved = await saveReq.promise;
                const dir = await dirReq.promise;
                if (file) openHandles.push(file);
                if (saved) openHandles.push(saved);
                if (dir) openHandles.push(dir);
                const openedText = file ? await file.readText().promise : "NULL";
                if (saved) await saved.writeText("saved-payload").promise;
                const savedBack = saved ? await saved.readText().promise : "NULL";
                const dirEntries = dir ? await dir.readDir().promise : [];
                setDialogStatus(
                  \`open=\${openedText},save=\${savedBack},dir=\${dirEntries.map(e => e.name).join(",")}\`,
                );

                // Persist, list, restore, and revoke bookmark on the picked folder.
                if (dir) {
                  const token = await persistHandle(dir).promise;
                  const beforeList = await listBookmarks().promise;
                  const restored = await restoreBookmark(token).promise;
                  openHandles.push(restored);
                  const restoredEntries =
                    restored.kind === "dir" ? await restored.readDir().promise : [];
                  const revoked = await revokeBookmark(token).promise;
                  const afterList = await listBookmarks().promise;
                  setBookmarkStatus(
                    \`tok=\${token.length > 0},before=\${beforeList.length},restored=\${restoredEntries.length},rev=\${revoked},after=\${afterList.length}\`,
                  );
                }
              })();
            }}
          >
            Run Dialogs & Bookmarks
          </Button>
        </row>
      </column>
    );
  }, root);
}
`;

const leakFileHandleAndWatcherSource = `
import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { openAuthorizedDirectory, watchDirectory } from "@hewel/zgui-services";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  // Open outside Solid owner so neither DirHandle nor watcher is cleaned up -> E_TEARDOWN!
  const dir = openAuthorizedDirectory("$XDG_DOCUMENTS_DIR/notes");
  watchDirectory(dir, () => {});
  return renderApp(() => (
    <column style="width: 100%; height: 100%; padding: 12px">
      <text testId="status">LEAKED</text>
    </column>
  ), root);
}
`;

test("local files preview covers manifest allowlist, symlink-escape rejection, scripted dialogs, bookmarks, watchDirectory, and clean teardown (#61)", async t => {
  const temp = await createTempWorkspace("preview-files-");
  t.after(() => temp.cleanup());

  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "files-preview",
    filesPreviewSource,
    {
      rawSource: true,
      capabilities: {
        files: {
          directories: [{ path: "$XDG_DOCUMENTS_DIR/notes", mode: "readwrite" }],
        },
      },
    },
  );
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "files-preview",
    runtimeEnvironment: {
      homeDir: "/home/preview",
      xdgDocumentsDir: "/home/preview/Documents",
    },
    services: {
      files: {
        dirs: ["/home/preview/Documents/notes", "/home/preview/workspace"],
        files: {
          "/home/preview/Documents/notes/welcome.md": "hello-welcome",
          "/home/preview/Documents/secret.txt": "outside-secret",
          "/home/preview/workspace/readme.md": "workspace-readme",
          "/home/preview/workspace/picked.md": "picked-content",
        },
        symlinks: {
          "/home/preview/Documents/notes/escape-link": "/home/preview/Documents/secret.txt",
        },
        dialogs: [
          { action: "openFile", path: "/home/preview/workspace/picked.md" },
          { action: "saveFile", path: "/home/preview/workspace/saved.md" },
          { action: "openDirectory", path: "/home/preview/workspace" },
        ],
      },
    },
    events: [],
  });

  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "files-preview",
    requiredCapabilities: [],
    steps: [
      { id: "settle-0", op: "clock.advance", byMs: 32 },
      {
        id: "click-dialogs",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "btn-dialogs" },
      },
      { id: "settle-1", op: "clock.advance", byMs: 32 },
      { id: "cap-done", op: "capture", captureId: "done", surface: "main" },
    ],
  });

  const tree = await readTree(out, "done");
  assert.equal(getNode(tree, "dialog-bg")?.text, "DENIED:E_CAPABILITY");
  assert.equal(getNode(tree, "unauth-dir")?.text, "DENIED:E_CAPABILITY");
  assert.equal(
    getNode(tree, "escape-status")?.text,
    "init=hello-welcome,final=hello-draft,entries=sub,welcome.md,sym=E_PATH_ESCAPE,trav=InvalidPath",
  );
  assert.equal(
    getNode(tree, "dialog-status")?.text,
    "open=picked-content,save=saved-payload,dir=picked.md,readme.md,saved.md",
  );
  assert.equal(
    getNode(tree, "bookmark-status")?.text,
    "tok=true,before=1,restored=3,rev=true,after=0",
  );
  const watchText = getNode(tree, "watch-log")?.text ?? "";
  assert.match(watchText, /create:sub/);
  assert.match(watchText, /rename:sub\/final\.md/);
  assert.equal(result.teardown.clean, true);
});

test("unclosed file handles and directory watchers fail teardown with E_TEARDOWN (#61)", async t => {
  const temp = await createTempWorkspace("preview-files-leak-");
  t.after(() => temp.cleanup());

  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "files-leak",
    leakFileHandleAndWatcherSource,
    {
      rawSource: true,
      capabilities: {
        files: {
          directories: [{ path: "$XDG_DOCUMENTS_DIR/notes", mode: "read" }],
        },
      },
    },
  );
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "files-leak",
    services: {
      files: {
        dirs: ["/home/preview/Documents/notes"],
      },
    },
    events: [],
  });

  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "files-leak",
    requiredCapabilities: [],
    allowFailure: true,
    steps: [
      { id: "settle-0", op: "clock.advance", byMs: 32 },
      { id: "cap-leak", op: "capture", captureId: "leak", surface: "main" },
    ],
  });

  assert.equal(result.exitCode, 9);
  assert.equal(result.firstError?.code, "E_TEARDOWN");
  assert.equal(result.teardown.clean, false);
  assert.equal(result.teardown.resources.fileHandles, 1);
  assert.equal(result.teardown.resources.fsWatchers, 1);
});
