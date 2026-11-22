// Compile the actual patched queue with the host's pinned dependency graph,
// without enrolling the entire standalone vendor workspace in host test runs.
#[path = "../../../vendor/zgui-runtime/src/timer.rs"]
mod timer;
