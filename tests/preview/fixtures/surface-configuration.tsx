import { Button, renderApp } from "@zgui/ui";
import type { NativeNode } from "@zgui/ui";
export const hostApiVersion = 1;
export function mount(root: NativeNode): () => void {
  return renderApp(() => <Button testId="trip" style={{ width: "100px", height: "40px" }}
    onClick={() => { throw new Error("configuration-action-dispatched"); }}>Trip</Button>, root);
}
