# zgui-js

Native applications and repeatable previews of their behavior.

## Language

**Mount region**: The portion of a native document owned by one application session at a time.

**Session**: One running application generation attached to a mount region. A retired session can no longer update that region.

**Bridge turn**: One bounded application entry permitted to mutate a mount region's native tree. Detached nodes become invalid when the turn completes, and an uncaught turn fault retires the session's ownership of that region.

**Session host**: The owner of session replacement, retirement, and recovery for its mount regions.

**Generation**: A monotonically increasing identity for a session incarnation, including incarnations that fail to mount.

**Last successful build**: The latest package that completed mounting and remains eligible for an explicit retry after a failure.

**Integration harness**: The native baseline and exercise workflow used to verify JavaScript updates, scheduling, and external wakes.

**Asset provider**: The authority that accepts an application's text-asset requests and supplies their outcomes. Live access and deterministic fixtures obey the same request and cancellation contract.

**Fixture runtime**: The run-wide owner of deterministic asset behavior, scheduled fixture events, and observed calls. Its history survives session replacement.

**Run ledger**: The ordered record of a preview run's attempted actions, captures, assertions, and failures. The earliest cause remains identifiable when later evidence publication or teardown also fails.

**Capture evidence**: The native tree and frame observations associated with a pixel presentation, including their build, session generation, and surface identity.

**Historical evidence**: The last complete capture evidence retained after a failure. It does not claim to satisfy the failing action's current state.
