import { createSignal, cx, renderApp, type NativeNode } from "@hewel/zgui";
import { Button, ThemeProvider, type ColorScheme, type ThemePreset } from "@hewel/zgui/components";
import styles from "./card.module.css";

export const hostApiVersion = 1;

function CssModulesDemo() {
  const [scheme, setScheme] = createSignal<ColorScheme>("light");
  const [preset, setPreset] = createSignal<ThemePreset>("base");
  const [active, setActive] = createSignal(false);

  return (
    <ThemeProvider scheme={scheme()} light={{ preset: preset() }} dark={{ preset: preset() }}>
      <column class="app-shell" testId="app-shell">
        <column
          testId="scoped-card"
          class={cx(styles.card)}
          data-active={active() ? "true" : "false"}
        >
          <label testId="card-title" class={styles.title}>
            {`CSS Modules (${scheme()}/${preset()})`}
          </label>
          <label testId="card-state">{active() ? "active" : "idle"}</label>
        </column>
        <row class={styles.toolbar}>
          <Button
            testId="btn-toggle-active"
            onClick={() => setActive(prev => !prev)}
          >
            Toggle Active
          </Button>
          <Button
            testId="btn-toggle-theme"
            onClick={() => {
              setScheme(prev => (prev === "light" ? "dark" : "light"));
              setPreset(prev => (prev === "base" ? "ocean" : "base"));
            }}
          >
            Switch Theme
          </Button>
          <Button
            testId="btn-layer-override"
            class={styles.overrideButton}
            onClick={() => setActive(true)}
          >
            Layered Override
          </Button>
        </row>
      </column>
    </ThemeProvider>
  );
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <CssModulesDemo />, root);
}
