import * as stylex from "@stylexjs/stylex";
import { Image, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Button, ThemeProvider } from "@hewel/zgui/components";
import { colors } from "./colors.stylex";

const pulse = stylex.keyframes({
  from: { opacity: 0.35 },
  to: { opacity: 1 },
});

const darkTheme = stylex.createTheme(colors, {
  surface: "#182230",
  text: "#f5f7fa",
  accent: "#89a2ff",
});

const styles = stylex.create({
  root: {
    width: "700px",
    padding: "16px",
    gap: "8px",
    backgroundColor: colors.surface,
    color: colors.text,
    fontFamily: "StyleX Fixture",
  },
  panel: { padding: "12px", backgroundColor: "#dbeafe" },
  compact: { padding: "4px" },
  track: {
    width: { default: "200px", "@media (max-width: 600px)": "150px" },
    height: "12px",
    backgroundColor: "#e5e7eb",
  },
  fill: { height: "100%", backgroundColor: colors.accent },
  fillWidth: (width: number) => ({ width }),
  baseOrder: { backgroundColor: "#22c55e" },
  control: {
    padding: "3px",
    backgroundColor: { default: "#374151", ":hover": "#dc2626", ":focus": "#2563eb" },
  },
  nested: { padding: "4px", backgroundColor: colors.surface, color: colors.text },
  responsiveFont: { fontFamily: "var(--responsive-family)" },
  image: { width: "16px", height: "16px" },
  animated: {
    width: "20px",
    height: "20px",
    backgroundColor: "#f59e0b",
    animationName: pulse,
    animationDuration: "var(--zui-motion-duration-normal)",
    animationTimingFunction: "linear",
    animationIterationCount: "1",
    animationFillMode: "both",
  },
  conflictTarget: { width: "8px", height: "8px", backgroundColor: "#111827" },
});

function StyleXExample() {
  const [progress, setProgress] = createSignal(50);
  const [compact, setCompact] = createSignal(false);
  const [dark, setDark] = createSignal(false);
  const [scheme, setScheme] = createSignal<"light" | "dark">("light");
  const [motion, setMotion] = createSignal("running");
  const [conflict, setConflict] = createSignal(false);
  const [nestedEnabled, setNestedEnabled] = createSignal(true);
  const [fontMismatch, setFontMismatch] = createSignal(false);
  const [schemeMismatch, setSchemeMismatch] = createSignal(false);
  return (
    <ThemeProvider scheme={scheme()}>
      <column {...stylex.props(styles.root, dark() && darkTheme)} testId="root" data-font-mismatch={fontMismatch() ? "true" : null} data-scheme-mismatch={schemeMismatch() ? "true" : null}>
        <row style={{ gap: "6px", "flex-wrap": "wrap" }}>
          <Button testId="progress" {...stylex.props(styles.control)}
            onClick={() => setProgress(value => value >= 200 ? 50 : value + 50)}>Progress</Button>
          <Button testId="compact" {...stylex.props(styles.control)} onClick={() => setCompact(value => !value)}>Compact</Button>
          <Button testId="dark" onClick={() => setDark(value => !value)}>Application theme</Button>
          <Button testId="nested" onClick={() => setNestedEnabled(value => !value)}>Nested theme</Button>
          <Button testId="scheme" onClick={() => setScheme(value => value === "light" ? "dark" : "light")}>SDK theme</Button>
          <Button testId="invalid" onClick={() => setProgress(Number.NaN)}>Invalid width</Button>
          <Button testId="conflict" onClick={() => setConflict(true)}>Alias conflict</Button>
          <Button testId="font-mismatch" onClick={() => setFontMismatch(true)}>Font mismatch</Button>
          <Button testId="scheme-font-mismatch" onClick={() => setSchemeMismatch(true)}>Scheme font mismatch</Button>
        </row>
        <text testId="progress-label">Width: {progress()}</text>
        <column {...stylex.props(styles.panel, compact() && styles.compact)} testId="panel">
          <box {...stylex.props(styles.track)} testId="track">
            <box {...stylex.props(styles.fill, styles.fillWidth(progress()))} testId="fill" />
          </box>
        </column>
        <box data-base-order="true" {...stylex.props(styles.baseOrder)} testId="base-order" />
        <column {...stylex.props(styles.nested, nestedEnabled() && darkTheme)} testId="nested-target">
          <text>Nested theme</text>
        </column>
        <Image src="assets/red.png" {...stylex.props(styles.image)} testId="package-image" />
        <box {...stylex.props(styles.animated)} testId="animated" onAnimationEnd={() => setMotion("ended")} />
        <text testId="motion">{motion()}</text>
        <text {...stylex.props(styles.responsiveFont)} testId="responsive-font">Responsive font</text>
        <box class={conflict() ? "manual" : null} {...stylex.props(styles.conflictTarget)} testId="conflict-target" />
      </column>
    </ThemeProvider>
  );
}

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <StyleXExample />, root);
}
