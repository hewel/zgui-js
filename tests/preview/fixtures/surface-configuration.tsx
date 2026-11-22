import { Button, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode): () => void {
  return renderApp(
    () => (
      <Button
        testId="trip"
        style={{ width: "100px", height: "40px" }}
        onClick={() => {
          throw new Error("configuration-action-dispatched");
        }}
      >
        Trip
      </Button>
    ),
    root,
  );
}
