import { closeWindow, createMemo, onCloseRequest, renderApp, useWindowState } from "@hewel/zgui";
import type { NativeKeyEvent, NativeNode } from "@hewel/zgui";
import {
  Badge,
  Button,
  Combobox,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Separator,
  ThemeProvider,
  ToastProvider,
} from "@hewel/zgui/components";
import type { SelectOption } from "@hewel/zgui/components";
import {
  Outlet,
  RouterProvider,
  createRouter,
  useLocation,
  useNavigate,
  useRouter,
} from "@hewel/zgui/router";
import { StudioContext, createStudioState, useStudio } from "./studio-context.js";
import { AboutPrefsWindow } from "./views/about-window.js";
import { EditorView } from "./views/editor-view.js";
import { SearchView } from "./views/search-view.js";
import { SettingsView } from "./views/settings-view.js";

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <MarkdownStudioRoot />, root);
}

function StudioShell() {
  const studio = useStudio();
  const router = useRouter();
  const location = useLocation();
  const navigate = useNavigate();
  const windowState = useWindowState();

  let closingWindow = false;
  const flushAndForceClose = () => {
    if (closingWindow) return;
    closingWindow = true;
    void studio
      .flushPrefs()
      .catch(() => {
        // Persistence errors are surfaced via studio.prefsError()
      })
      .finally(() => {
        closeWindow({ force: true });
      });
  };

  onCloseRequest(event => {
    if (studio.hasUnsavedChanges()) {
      event.preventDefault();
      studio.setCloseDialogOpen(true);
      return;
    }
    event.preventDefault();
    flushAndForceClose();
  });

  const docOptions = createMemo<readonly SelectOption[]>(() =>
    studio.docs().map(doc => ({
      value: doc.id,
      label: `${doc.title}${doc.dirty ? " *" : ""}`,
      description: doc.relativePath ?? doc.badge,
    })),
  );

  const handleGlobalKeyDown = (event: NativeKeyEvent) => {
    if (!event.modifiers.control) return;
    const ch = event.character?.toLowerCase();
    if (ch === "s") {
      event.preventDefault();
      void studio.saveActiveDocument();
    } else if (ch === "o") {
      event.preventDefault();
      void studio.openLocalFile();
    } else if (ch === "f" && event.modifiers.shift) {
      event.preventDefault();
      void navigate("/search");
    }
  };

  const routeActive = (path: string) =>
    location().pathname === path || (path === "/editor" && location().pathname === "/");

  return (
    <ThemeProvider
      scheme={studio.scheme()}
      light={{ preset: studio.preset() }}
      dark={{ preset: studio.preset() }}
    >
      <column class="mde-root" testId="md-editor-root" onKeyDown={handleGlobalKeyDown}>
        <row class="mde-topbar">
          <row style={{ gap: "6px", "align-items": "center" }}>
            <Button
              testId="nav-back"
              size="xs"
              variant="outline"
              disabled={!router.canGoBack()}
              onClick={() => void router.back()}
              ariaLabel="Back"
            >
              ←
            </Button>
            <Button
              testId="nav-forward"
              size="xs"
              variant="outline"
              disabled={!router.canGoForward()}
              onClick={() => void router.forward()}
              ariaLabel="Forward"
            >
              →
            </Button>
            <Separator orientation="vertical" />
            <Button
              testId="nav-editor"
              size="xs"
              variant={routeActive("/editor") ? "default" : "outline"}
              onClick={() => void navigate("/editor")}
            >
              Editor
            </Button>
            <Button
              testId="nav-search"
              size="xs"
              variant={routeActive("/search") ? "default" : "outline"}
              onClick={() => void navigate("/search")}
            >
              Search
            </Button>
            <Button
              testId="nav-settings"
              size="xs"
              variant={routeActive("/settings") ? "default" : "outline"}
              onClick={() => void navigate("/settings")}
            >
              Settings
            </Button>
            <Separator orientation="vertical" />
            <Combobox
              testId="quick-doc-combo"
              inputTestId="quick-doc-input"
              listTestId="quick-doc-list"
              options={docOptions()}
              value={studio.activeDoc().id}
              onValueChange={docId => {
                studio.setActiveDocId(docId);
                if (!routeActive("/editor")) void navigate("/editor");
              }}
              placeholder="Switch doc..."
              label="Quick document switcher"
              style={{ width: "180px" }}
            />
            <Badge
              testId="save-state-badge"
              variant={studio.activeDoc().dirty ? "default" : "secondary"}
            >
              {studio.activeDoc().dirty ? "Unsaved *" : "Saved"}
            </Badge>
          </row>

          <row style={{ gap: "6px", "align-items": "center" }}>
            <Button
              testId="btn-open-file"
              size="xs"
              variant="outline"
              onClick={() => void studio.openLocalFile()}
            >
              Open
            </Button>
            <Button
              testId="btn-save-file"
              size="xs"
              variant="outline"
              onClick={() => void studio.saveActiveDocument()}
            >
              Save
            </Button>
            <Button
              testId="btn-open-folder"
              size="xs"
              variant="outline"
              onClick={() => void studio.openWorkspaceFolder()}
            >
              Folder
            </Button>
            <Separator orientation="vertical" />
            <Button
              testId="btn-about-window"
              size="xs"
              variant="outline"
              onClick={() => studio.setPrefsWindowOpen(!studio.prefsWindowOpen())}
            >
              {studio.prefsWindowOpen() ? "Close About" : "About"}
            </Button>
            <Button
              testId="btn-toggle-scheme"
              size="xs"
              variant="outline"
              onClick={() => studio.setScheme(studio.scheme() === "dark" ? "light" : "dark")}
            >
              {studio.scheme() === "dark" ? "Light" : "Dark"}
            </Button>
          </row>
        </row>

        <box class="mde-outlet">
          <Outlet />
        </box>

        <row class="mde-statusbar">
          <row style={{ gap: "10px", "align-items": "center" }}>
            <text testId="status-msg">{studio.statusMsg()}</text>
            <text class="mono" testId="window-title-readout">
              {windowState().title}
            </text>
          </row>
          <row style={{ gap: "10px", "align-items": "center" }}>
            <text class="mono" testId="route-readout">
              {location().pathname}
            </text>
            <text
              class="mono"
              testId="watch-event-readout"
            >{`watch:${studio.externalEventLog()}`}</text>
            <text class="mono">
              {`${studio.stats().words} words · ${studio.stats().lines} lines · ${studio.stats().chars} chars`}
            </text>
          </row>
        </row>

        <Dialog open={studio.closeDialogOpen()} onOpenChange={studio.setCloseDialogOpen}>
          <DialogContent testId="unsaved-close-dialog">
            <DialogHeader>
              <DialogTitle>Unsaved Changes</DialogTitle>
              <DialogDescription>
                {`"${studio.activeDoc().title}" has unsaved edits. Save before closing Markdown Studio?`}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button
                testId="btn-close-cancel"
                variant="outline"
                onClick={() => studio.setCloseDialogOpen(false)}
              >
                Cancel
              </Button>
              <Button
                testId="btn-close-discard"
                variant="destructive"
                onClick={() => {
                  studio.setCloseDialogOpen(false);
                  flushAndForceClose();
                }}
              >
                Discard & Close
              </Button>
              <Button
                testId="btn-close-save"
                onClick={() => {
                  void studio.saveActiveDocument().then(ok => {
                    if (ok) {
                      studio.setCloseDialogOpen(false);
                      flushAndForceClose();
                    }
                  });
                }}
              >
                Save & Close
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>

        <AboutPrefsWindow />
        <ToastProvider corner="bottom-right" regionTestId="studio-toast-region" />
      </column>
    </ThemeProvider>
  );
}

function MarkdownStudioRoot() {
  const studio = createStudioState();
  const router = createRouter(
    [
      {
        path: "/",
        component: StudioShell,
        children: [
          { path: "", component: EditorView },
          { path: "editor", component: EditorView },
          { path: "search", component: SearchView },
          { path: "settings", component: SettingsView },
        ],
      },
    ] as const,
    { initialPath: "/editor" },
  );

  return (
    <StudioContext value={studio}>
      <RouterProvider router={router} />
    </StudioContext>
  );
}
