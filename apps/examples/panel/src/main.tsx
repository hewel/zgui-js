import { For, Show, createMemo, createSignal, onCleanup, onMount, renderApp } from "@zgui/ui";
import type { NativeNode } from "@zgui/ui";
import { watchNiri } from "@zgui/services";
import type { HostServiceError, HostSubscription, NiriSnapshot, NiriWindow, NiriWorkspace } from "@zgui/services";

function WorkspaceRow(props: { id: string; workspace: NiriWorkspace | undefined }) {
  return <column class={props.workspace?.focused ? "panel-row panel-focused" : "panel-row"}>
    <text class="panel-row-title">
      {props.workspace?.index}: {props.workspace?.name ?? "Unnamed workspace"}
    </text>
    <text class="panel-muted">{props.workspace?.output ?? "No output"} · ID {props.id}</text>
    <text class={props.workspace?.urgent ? "panel-urgent" : "panel-muted"}>
      {props.workspace?.focused ? "Focused" : props.workspace?.active ? "Active on output" : "Inactive"}
      {props.workspace?.urgent ? " · Urgent" : ""}
    </text>
    <text class="panel-muted">
      {props.workspace?.activeWindowId === null || props.workspace?.activeWindowId === undefined
        ? "No active window" : `Active window ${props.workspace.activeWindowId}`}
    </text>
  </column>;
}

function WindowRow(props: { id: string; window: NiriWindow | undefined; workspace: NiriWorkspace | undefined }) {
  return <column class={props.window?.focused ? "panel-row panel-focused" : "panel-row"}>
    <text class="panel-row-title">{props.window?.title ?? "Untitled window"}</text>
    <text class="panel-muted">
      {props.window?.appId ?? "Unknown application"} · ID {props.id}
      {props.window?.pid === null || props.window?.pid === undefined ? " · PID unavailable" : ` · PID ${props.window.pid}`}
    </text>
    <text class="panel-muted">
      {props.workspace ? `Workspace ${props.workspace.index}${props.workspace.name === null ? "" : `: ${props.workspace.name}`}`
        : props.window?.workspaceId === null || props.window?.workspaceId === undefined
          ? "No workspace" : `Workspace ID ${props.window.workspaceId}`}
    </text>
    <text class={props.window?.urgent ? "panel-urgent" : "panel-muted"}>
      {props.window?.focused ? "Focused" : "Unfocused"}
      {props.window?.floating ? " · Floating" : " · Tiled"}
      {props.window?.urgent ? " · Urgent" : ""}
    </text>
  </column>;
}

function Panel() {
  const [snapshot, setSnapshot] = createSignal<NiriSnapshot>();
  const [error, setError] = createSignal<HostServiceError>();
  const [startError, setStartError] = createSignal<string>();
  let subscription: HostSubscription | undefined;

  const workspaces = createMemo(() => new Map<string, NiriWorkspace>(snapshot()?.workspaces.map(workspace => [workspace.id, workspace])));
  const windows = createMemo(() => new Map<string, NiriWindow>(snapshot()?.windows.map(window => [window.id, window])));
  // For keys are primitive IDs, not the new DTO objects allocated on every delivery.
  const workspaceIds = createMemo(() => snapshot()?.workspaces.map(workspace => workspace.id) ?? []);
  const windowIds = createMemo(() => snapshot()?.windows.map(window => window.id) ?? []);
  const failed = () => error() !== undefined || startError() !== undefined;

  onCleanup(() => {
    subscription?.cancel();
    subscription = undefined;
  });
  onMount(() => {
    try {
      subscription = watchNiri(setSnapshot, setError);
    } catch (failure) {
      setStartError(failure instanceof Error ? failure.message : String(failure));
    }
  });

  return <column class="panel">
    <row class="panel-header">
      <text class="panel-heading">niri desktop</text>
      <text class={failed() ? "panel-status panel-error" : snapshot() ? "panel-status panel-live" : "panel-status panel-muted"}>
        {failed() ? "Disconnected" : snapshot() ? "Live" : "Waiting"}
      </text>
    </row>
    <Show when={failed()}>
      <column class="panel-notice">
        <text class="panel-error">{error() ? `${error()!.code}: ${error()!.message}` : startError()}</text>
        <text class="panel-muted">
          {snapshot() ? "Last snapshot retained; updates have stopped. " : "No snapshot received. "}
          Check niri and the host's NIRI_SOCKET, then remount to reconnect.
        </text>
      </column>
    </Show>
    <Show when={!failed() && !snapshot()}>
      <text class="panel-notice panel-muted">Waiting for initial workspace and window state from niri.</text>
    </Show>
    <Show when={snapshot()}>
      <row class="panel-content">
        <column class="panel-workspaces">
          <text class="panel-section-heading">Workspaces ({workspaceIds().length})</text>
          <scroll class="panel-list">
            <column class="panel-rows">
              <For each={workspaceIds()} fallback={<text class="panel-empty panel-muted">No workspaces reported.</text>}>
                {id => <WorkspaceRow id={id} workspace={workspaces().get(id)} />}
              </For>
            </column>
          </scroll>
        </column>
        <column class="panel-windows">
          <text class="panel-section-heading">Windows ({windowIds().length})</text>
          <scroll class="panel-list">
            <column class="panel-rows">
              <For each={windowIds()} fallback={<text class="panel-empty panel-muted">No open windows.</text>}>
                {id => <WindowRow id={id} window={windows().get(id)}
                  workspace={workspaces().get(windows().get(id)?.workspaceId ?? "")} />}
              </For>
            </column>
          </scroll>
        </column>
      </row>
    </Show>
  </column>;
}

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <Panel />, root);
}
