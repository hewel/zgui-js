import { Button, TextField, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode): () => void {
  return renderApp(
    () => (
      <column>
        <Button
          testId="pointer"
          onClick={() => {
            throw new Error("final-pointer-dispatched");
          }}
        >
          Pointer
        </Button>
        <Button
          testId="keyboard"
          onKeyDown={() => {
            throw new Error("final-key-dispatched");
          }}
        >
          Keyboard
        </Button>
        <TextField
          testId="text"
          style={{ width: "200px", height: "32px" }}
          onInput={() => {
            throw new Error("final-text-dispatched");
          }}
        />
      </column>
    ),
    root,
  );
}
