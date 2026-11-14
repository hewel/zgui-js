# @zgui/ui

Native TSX components and reactive primitives for the zgui host. Use the package's
`tsconfig.native.json` and `@zgui/ui` JSX import source; these are native nodes,
not browser elements. `renderApp` owns the mounted subtree and returns its
idempotent disposer.

## Native text controls

```tsx
import { TextField, TextArea, createSignal } from "@zgui/ui";

function Form() {
  const [name, setName] = createSignal("A中B");
  return <column>
    <TextField ariaLabel="Name" value={name()}
      onInput={event => setName(event.value)} />
    <TextArea ariaLabel="Notes" defaultValue={"one\n二"} />
  </column>;
}
```

`TextField` creates a single-line native `field`; `TextArea` creates a multiline
native `editor`. Both default to `tabIndex={0}` and fix their semantic role to
`textInput` and `multilineTextInput`, respectively. Intrinsic `<field />` and
`<editor />` accept `NativeTextControlProps`, including a generic native `role`.
Component props are exported as `TextFieldProps` and `TextAreaProps`.

The native host supplies `white-space: pre` for `field` and `pre-wrap` for
`editor`, matching zgui's native text controls. These defaults preserve spaces
and multiline layout across reloads; application CSS can override them. Other
visual styling remains application-owned.

- `value` reacts to its source and writes through the native value API, never
  through reconciled text children. Echo `event.value` into a controlled signal
  in `onInput`. The native model owns text, caret, selection, composition and
  editing behavior; the SDK does not implement another keyboard/IME model.
- `value={null}` explicitly sets an empty string. An initially `undefined` value
  is absent under Solid's initial property diff, so `defaultValue` may initialize
  the control. Changing a previously assigned value to `undefined` (or removing
  it from a spread) clears it. A direct renderer `setProp` assignment of
  `undefined` also clears it.
- `defaultValue` is initial-only: its first actual assignment initializes the
  native value, and later changes are ignored. Once any explicit `value`
  assignment occurs, `defaultValue` cannot overwrite it, regardless of prop
  order. Use `value` for programmatic resets.
- `readOnly` updates native read-only state and accessibility semantics. A
  read-only field remains focusable and selectable; `disabled` uses native
  disabled behavior. `disabled`, `ariaLabel` and `ariaDescription` remain reactive.
- `onInput` reports native edits; `onChange` separately reports committed changes.
  Programmatic value assignments do not synthesize input events. The native
  model decides caret preservation for equal-value controlled echoes.
- Event `value` and `selection` are owned snapshots. Selection `start` and `end`
  are **UTF-8 byte offsets**, not JavaScript UTF-16 indices. Selection is read-only;
  no selection setter or measurement API is exposed. Event targets are native
  handles, not browser elements.
- Children are forbidden in the JSX types and at runtime. Components reject a
  `children` property before evaluating its getter. Native editor-owned
  descendants are opaque to the bridge and must not be reconciled or mutated by
  application code. Use `value`/`defaultValue`, not nested text.

Browser-only props such as `type`, `rows`, `selectionStart` and `className` are not
accepted. Use native `class`, native style strings/objects, and the supported
native event handlers.

## Editing example

Build with `pnpm ui:build --app editing`, or watch with
`pnpm ui:dev --app editing`. The example uses real native controls and ordinary
TSX event handlers, without test hooks or a simulated bridge.

| Class | Initial state / action |
| --- | --- |
| `editing-field` | Controlled `TextField`, `A中B` |
| `editing-area` | Controlled `TextArea`, `one\n二` |
| `editing-default` | Uncontrolled `TextField`, initial `seed` |
| `editing-readonly` | Read-only `TextField`, `Read only` |
| `editing-disabled` | Disabled `TextField`, `Disabled` |
| `editing-reset` | Button writes `Reset 中` to the controlled field |
| `editing-change-default` | Button changes the default source to `replacement`; current uncontrolled text stays unchanged |

The visible event report aggregates all five controls. Inputs in the two editable
controlled fields echo into their corresponding signals; every input increments
`editing-input-count`. Committed changes increment `editing-change-count`
separately. Both event kinds update `editing-last-value` and
`editing-last-selection` (`start:end`, UTF-8 bytes). `editing-target-valid` starts
as `true` and remains true only if every received input/change event has
`event.target === event.currentTarget`.

Report values are plain native text under those stable classes. Initial counts
are `0`, last value is empty, selection is `0:0`, and target validity is `true`.
Read-only and disabled controls have handlers too, so an unexpected edit is
observable rather than hidden. Native focus borders and disabled styles make
keyboard focus and non-editable states visible.
