# Async status

Build with `pnpm build --app async`, or develop with `pnpm dev --app async`.
The app uses `@hewel/zgui` lifecycle hooks and public scheduling globals. No input
or harness call is needed after mounting.

Each status is a separate native text node. Initial text is:

- `Async status`
- `Timer: pending`
- `Frame: pending`
- `Promise: pending`
- `Service: waiting for timer`

A native frame callback sets `Frame: fired`. Its Promise continuation sets
`Promise: layout updated` and inserts a 32px-high `Promise layout row` text node,
so the continuation changes the upcoming native layout, not just a JS variable.
A 50ms minimum-delay timer sets `Timer: fired`, starts
`readTextAsset('assets/message.txt')`, and sets `Service: reading`. The Rust task
reads the real packaged asset; its completion sets `Service: Rust task completed`.
The builder declares `assets/message.txt` in the generated manifest. No worker
sleep or fake service result is used. Timer and frame ordering is not assumed.

Final stable native text is:

- `Async status`
- `Timer: fired`
- `Frame: fired`
- `Promise: layout updated`
- `Service: Rust task completed`
- `Promise layout row`

Disposal marks the component inactive, clears a pending timer, cancels a pending
frame and cancels a pending service request. Promise success/rejection handlers
are installed immediately; neither frame jobs nor service continuations update
disposed UI. Fired registrations and settled requests release their local
handles. The renderApp disposer owns subtree and reactive cleanup.
