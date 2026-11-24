import {
  LayerSurface,
  PopupSurface,
  WindowSurface,
  createProjection,
  createSpring,
  createStore,
  cx,
  renderApp,
} from "@hewel/zgui";
import type {
  BlurRegion,
  BlurRegionNodes,
  NativeNode,
  ProjectionOptions,
  SpringOptions,
  SpringPhysicsOptions,
  StoreOptions,
  SurfaceInsets,
} from "@hewel/zgui";
import { Button, Slider, ThemeProvider } from "@hewel/zgui/components";
import {
  MenuCheckboxItemPrimitive,
  MenuItemPrimitive,
  MenuListPrimitive,
  MenuRadioGroupPrimitive,
  MenuRadioItemPrimitive,
  MenuRootPrimitive,
  SliderPrimitive,
  createOverlayState,
  createPresence,
  createRetainedPresence,
  createRovingFocus,
} from "@hewel/zgui/primitives";
import type {
  RetainedPresenceOptions,
  SliderPrimitiveProps,
  SliderPrimitiveRenderState,
} from "@hewel/zgui/primitives";
import { createRouter, RouterProvider, Outlet } from "@hewel/zgui/router";
import {
  createHostQuery,
  createKvState,
  httpRequest,
  openKvStore,
  readTextAsset,
  spawnWorker,
} from "@hewel/zgui-services";
import type { HostQueryOptions, KvStateOptions } from "@hewel/zgui-services";

const maybeDisabled: boolean | undefined = undefined;
const maybeChecked: boolean | undefined = undefined;
const maybeClass: string | undefined = undefined;
const maybeOpen: boolean | undefined = undefined;
const maybeBlur: BlurRegion | undefined = undefined;
const maybeBlurNodes: BlurRegionNodes = { nodes: [], minOpacity: undefined };
const maybeBlurAsRegion: BlurRegion = maybeBlurNodes;
void maybeBlurAsRegion;
const maybeInsets: SurfaceInsets | undefined = { top: undefined, left: 24 };

export function strictOptionalProbe(root: NativeNode) {
  const router = createRouter([{ path: "/", component: () => <text>home</text> }]);
  const overlay = createOverlayState({ open: maybeOpen, defaultOpen: false });
  const presence = createPresence(() => overlay.open());
  const springPhysics: SpringPhysicsOptions = {
    stiffness: undefined,
    damping: undefined,
    mass: undefined,
    precision: undefined,
    velocityPrecision: undefined,
  };
  const springOpts: SpringOptions = { ...springPhysics, initial: undefined };
  const spring = createSpring(() => (overlay.open() ? 1 : 0), springOpts);
  spring.configure({ stiffness: undefined, damping: undefined });
  const retainedOpts: RetainedPresenceOptions = { settled: undefined };
  const retained = createRetainedPresence(() => (overlay.open() ? "ok" : null), retainedOpts);
  const storeOpts: StoreOptions = { name: undefined, loc: undefined, shallow: undefined };
  const projOpts: ProjectionOptions = {
    name: undefined,
    loc: undefined,
    shallow: undefined,
    key: undefined,
    seedLoadingValue: undefined,
  };
  const [strictStore] = createStore({ count: 0 }, storeOpts);
  const strictProj = createProjection(
    draft => {
      draft.count = strictStore.count;
    },
    { count: 0 },
    projOpts,
  );
  const kvOpts: KvStateOptions<{ readonly count: number }> = {
    validate: raw => raw as { readonly count: number },
    version: undefined,
    debounceMs: undefined,
  };
  const strictKv = createKvState(openKvStore("main", "strict"), "k", { count: 0 }, kvOpts);
  const queryOpts: HostQueryOptions<string> = {
    initialValue: undefined,
    keepPreviousData: undefined,
    onError: undefined,
  };
  const strictQuery = createHostQuery(
    () => (overlay.open() ? "assets/a.txt" : false),
    path => readTextAsset(path),
    queryOpts,
  );
  const roving = createRovingFocus(() => "vertical");
  void [
    presence.mounted(),
    spring.settled(),
    retained.present(),
    retained.value(),
    strictProj.count,
    strictKv.value(),
    strictQuery.data(),
    roving.active(),
  ];
  void httpRequest;
  void spawnWorker;

  return renderApp(
    () => (
      <ThemeProvider>
        <RouterProvider router={router}>
          <box class={cx("base", maybeClass)}>
            <Button disabled={maybeDisabled}>Save</Button>
            <Slider
              value={undefined}
              defaultValue={undefined}
              onValueChange={undefined}
              min={undefined}
              max={undefined}
              step={undefined}
              pointerStep={undefined}
              orientation={undefined}
              label={undefined}
              disabled={maybeDisabled}
              onDraggingChange={undefined}
            />
            <SliderPrimitive
              {...({
                value: undefined,
                defaultValue: undefined,
                onValueChange: undefined,
                min: undefined,
                max: undefined,
                step: undefined,
                pointerStep: undefined,
                orientation: undefined,
                label: undefined,
                disabled: maybeDisabled,
                onDraggingChange: undefined,
              } satisfies SliderPrimitiveProps)}
            >
              {({ value, fraction, dragging, trackRef }: SliderPrimitiveRenderState) => (
                <box ref={trackRef}>
                  <text>{`${value()}-${fraction()}-${dragging()}`}</text>
                </box>
              )}
            </SliderPrimitive>
            <MenuRootPrimitive open={maybeOpen} defaultOpen={false}>
              <MenuListPrimitive>
                <MenuItemPrimitive disabled={maybeDisabled} onSelect={() => {}}>
                  Open
                </MenuItemPrimitive>
                <MenuCheckboxItemPrimitive checked={maybeChecked} disabled={maybeDisabled}>
                  Wrap
                </MenuCheckboxItemPrimitive>
                <MenuRadioGroupPrimitive value="a">
                  <MenuRadioItemPrimitive value="a" disabled={maybeDisabled}>
                    Option A
                  </MenuRadioItemPrimitive>
                </MenuRadioGroupPrimitive>
              </MenuListPrimitive>
            </MenuRootPrimitive>
            <Outlet />
            <WindowSurface
              name="glass-win"
              title="Glass"
              width={300}
              height={200}
              blurRegion={maybeBlur}
            />
            <LayerSurface name="glass-layer" blurRegion={maybeBlur} />
            <PopupSurface
              name="glass-pop"
              parent="glass-layer"
              anchorRect={{ x: 0, y: 0, width: 10, height: 10 }}
              width={100}
              blurRegion={maybeBlur}
              geometryInsets={maybeInsets}
            />
          </box>
        </RouterProvider>
      </ThemeProvider>
    ),
    root,
  );
}
