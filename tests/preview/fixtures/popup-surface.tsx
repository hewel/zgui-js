import { createSignal, LayerSurface, PopupSurface, renderApp, Show } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [barOpen, setBarOpen] = createSignal(true);
    const [declared, setDeclared] = createSignal(true);
    const [menuDismissals, setMenuDismissals] = createSignal(0);
    const [submenuDismissals, setSubmenuDismissals] = createSignal(0);
    const [offset, setOffset] = createSignal(0);
    return <column>
      <text testId="dismissals">{`${menuDismissals()}:${submenuDismissals()}`}</text>
      <control testId="move-anchor" onClick={() => setOffset(value => value + 10)}>Move anchor</control>
      <control testId="remove-menus" onClick={() => setDeclared(false)}>Remove menu declarations</control>
      <control testId="declare-menus" onClick={() => setDeclared(true)}>Declare menus</control>
      <control testId="remove-bar" onClick={() => setBarOpen(false)}>Remove bar</control>
      <control testId="restore-bar" onClick={() => setBarOpen(true)}>Restore bar</control>
      <Show when={declared()}>
        <PopupSurface name="submenu" parent="menu" anchorRect={{ x: 40, y: 8, width: 60, height: 28 }}
          placement={{ anchor: "right", gravity: "right" }} width={88} height={72}
          onDismiss={() => setSubmenuDismissals(value => value + 1)}>
          <text testId="content">Nested menu</text>
        </PopupSurface>
        <PopupSurface name="menu" parent="bar" anchorRect={{ x: 8, y: 0, width: 64, height: 28 }}
          width={120 + offset()} height={96} offset={{ x: offset(), y: 0 }}
          onDismiss={() => setMenuDismissals(value => value + 1)}>
          <text testId="content">Bar menu</text>
        </PopupSurface>
      </Show>
      <Show when={barOpen()}>
        <LayerSurface name="bar" output="right" anchor={["top", "left", "right"]} height={28}>
          <text testId="content">Bar</text>
        </LayerSurface>
      </Show>
    </column>;
  }, root);
}
