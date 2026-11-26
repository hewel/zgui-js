# zgui-js

Native applications and repeatable previews of their behavior.

## Language

**Mount region**: The portion of a native document owned by one application session at a time.

**Session**: One running application generation owning zero or more mount regions. A retired session can no longer update any of its regions.

**Bridge turn**: One bounded application entry permitted to mutate its session's mount regions. Detached nodes become invalid when the turn completes, and an uncaught turn fault retires the session's ownership of its regions.

**Session host**: The owner of session replacement, retirement, and recovery for its mount regions.

**Generation**: A monotonically increasing identity for a session incarnation, including incarnations that fail to mount.

**Last successful build**: The latest package that completed mounting and remains eligible for an explicit retry after a failure.

**Integration harness**: The native baseline and exercise workflow used to verify JavaScript updates, scheduling, and external wakes.

**Asset provider**: The authority that accepts an application's text-asset requests and supplies their outcomes. Live access and deterministic fixtures obey the same request and cancellation contract.

**Fixture runtime**: The run-wide owner of deterministic asset behavior, scheduled fixture events, and observed calls. Its history survives session replacement.

**Run ledger**: The ordered record of a preview run's attempted actions, captures, assertions, and failures. The earliest cause remains identifiable when later evidence publication or teardown also fails.

**Capture evidence**: The native tree and frame observations associated with a pixel presentation, including their build, session generation, and surface identity.

**Historical evidence**: The last complete capture evidence retained after a failure. It does not claim to satisfy the failing action's current state.

**Artifact inventory**: The sealed manifest of files, roles, sizes, and content digests owned by one preview output directory, verified before a run or comparison result is accepted as complete.

**Package publication**: An immutable candidate package and its build-attempt status exposed by the builder before a host evaluates it.
_Avoid_: Generation (reserved for session incarnations), native acceptance

**Native acceptance**: The outcome of preflighting and mounting one published package for a mount region. Publication alone does not make a candidate accepted or replace the last successful application.

**Stylesheet revision**: The identity of the stylesheet-set state consumed by a native document. Class and inline-declaration changes retain this identity; applied stylesheet installation, replacement, and removal change it.

**Style profile**: The versioned agreement between compiled application stylesheets and the native CSS behaviors a host accepts.
_Avoid_: StyleX profile

**Shell surface**: A desktop-managed layer, session-lock surface, or anchored popup backing its own mount region within the owning session.
_Avoid_: Panel window, popup window

**Input region**: The subset of a surface's area that receives pointer events while letting events elsewhere pass through to surfaces beneath.

**Session lock**: An exclusive compositor lock under which only lock surfaces are presented across outputs until explicitly unlocked.

**Capability**: A package-declared authority permitting a session to open shell surfaces, lock the desktop, connect to external endpoints, or read host media.

**Image handle**: A session-scoped opaque reference to host-held image bytes outside the JavaScript heap.
