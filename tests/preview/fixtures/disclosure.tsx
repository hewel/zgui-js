import { For, Show, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode, NativeStyle } from "@hewel/zgui";
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
  Button,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  Input,
  ThemeProvider,
} from "@hewel/zgui/components";
export const hostApiVersion = 1;
function Counter(props: { id: string }) {
  const [count, setCount] = createSignal(0);
  return (
    <Button testId={props.id} size="sm" onClick={() => setCount(n => n + 1)}>
      Count: {count()}
    </Button>
  );
}
export function mount(root: NativeNode): () => void {
  return renderApp(() => {
    const [open, setOpen] = createSignal(false);
    const [height, setHeight] = createSignal(96);
    const [style, setStyle] = createSignal<NativeStyle>({
      "background-color": "var(--zui-color-muted)",
    });
    const [shown, setShown] = createSignal(true);
    const [dark, setDark] = createSignal(false);
    const [values, setValues] = createSignal<readonly string[]>(["a"]);
    const [multiple, setMultiple] = createSignal(false);
    const [empty, setEmpty] = createSignal(false);
    const [order, setOrder] = createSignal(["a", "b", "c"]);
    const [disabled, setDisabled] = createSignal(false);
    return (
      <ThemeProvider scheme={dark() ? "dark" : "light"}>
        <column
          style={{
            width: "720px",
            padding: "12px",
            gap: "8px",
            "background-color": "var(--zui-color-background)",
            color: "var(--zui-color-foreground)",
          }}
        >
          <text style={{ "font-size": "20px", "font-weight": "600" }}>Disclosures</text>
          <row style={{ gap: "8px" }}>
            <Button testId="grow" size="sm" onClick={() => setHeight(144)}>
              Grow
            </Button>
            <Button
              testId="restyle"
              size="sm"
              onClick={() => setStyle("background-color:var(--zui-color-accent);")}
            >
              Restyle
            </Button>
            <Button testId="show" size="sm" onClick={() => setShown(v => !v)}>
              Show/hide
            </Button>
            <Button testId="theme" size="sm" onClick={() => setDark(v => !v)}>
              Theme
            </Button>
            <Button testId="multiple" size="sm" onClick={() => setMultiple(v => !v)}>
              Multi
            </Button>
            <Button testId="empty" size="sm" onClick={() => setEmpty(v => !v)}>
              Empty
            </Button>
            <Button testId="order" size="sm" onClick={() => setOrder(["c", "a", "b"])}>
              Order
            </Button>
            <Button testId="disable" size="sm" onClick={() => setDisabled(v => !v)}>
              Disable
            </Button>
          </row>
          <row style={{ gap: "16px" }}>
            <text testId="c-state">{open() ? "open" : "closed"}</text>
            <text testId="acc-state">{values().join(",") || "none"}</text>
            <text testId="mode-state">{multiple() ? "multiple" : "single"}</text>
          </row>
          <Show when={shown()}>
            <row testId="scene" style={{ gap: "16px", "align-items": "flex-start" }}>
              <column style={{ width: "340px", "flex-shrink": "0", gap: "8px" }}>
                <Button testId="before" size="sm">
                  Before details
                </Button>
                <Collapsible open={open()} onOpenChange={setOpen}>
                  <CollapsibleTrigger testId="details-trigger">
                    Persistent details
                  </CollapsibleTrigger>
                  <CollapsibleContent testId="details-content" style={style()}>
                    <column
                      testId="details-body"
                      style={{
                        height: `${height()}px`,
                        "flex-shrink": "0",
                        padding: "8px",
                        gap: "8px",
                      }}
                    >
                      <Input
                        testId="details-input"
                        defaultValue="seed"
                        label="Saved detail"
                        onKeyDown={event => {
                          if (event.key === "Escape") {
                            setOpen(false);
                            event.preventDefault();
                          }
                        }}
                      />
                      <Counter id="details-count" />
                    </column>
                  </CollapsibleContent>
                </Collapsible>
                <Button testId="after" size="sm">
                  After details
                </Button>
              </column>
              <column style={{ width: "340px", "flex-shrink": "0", gap: "8px" }}>
                <Button testId="before-accordion" size="sm">
                  Before accordion
                </Button>
                <Accordion
                  testId="accordion"
                  value={values()}
                  onValueChange={setValues}
                  selection={multiple() ? "multiple" : "single"}
                  collapsible={empty()}
                  disabled={disabled()}
                >
                  <For each={order()}>
                    {id => (
                      <AccordionItem value={id} testId={`acc-item-${id}`} disabled={id === "b"}>
                        <AccordionTrigger testId={`acc-${id}`} level={2}>
                          <text>Section {id.toUpperCase()}</text>
                        </AccordionTrigger>
                        <AccordionContent testId={`acc-content-${id}`}>
                          <Show
                            when={id === "a"}
                            fallback={
                              <box style={{ height: "40px" }}>
                                <Counter id={`acc-count-${id}`} />
                              </box>
                            }
                          >
                            <Collapsible defaultOpen>
                              <CollapsibleTrigger testId="nested-trigger">
                                Nested details
                              </CollapsibleTrigger>
                              <CollapsibleContent testId="nested-content">
                                <box
                                  style={{
                                    height: "40px",
                                    padding: "8px",
                                    "background-color": "var(--zui-color-muted)",
                                  }}
                                >
                                  <text>Independent measurement</text>
                                </box>
                              </CollapsibleContent>
                            </Collapsible>
                          </Show>
                        </AccordionContent>
                      </AccordionItem>
                    )}
                  </For>
                </Accordion>
              </column>
            </row>
          </Show>
        </column>
      </ThemeProvider>
    );
  }, root);
}
