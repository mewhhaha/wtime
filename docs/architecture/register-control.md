# Register control transfers

P060 extends the opt-in integer Register executor with whole-function
`br_table` lowering. P130 supplies simultaneous edge moves; P061 supplies
integer execution and the common source-boundary integration. Decoded
remains the default and Portable policy.

Each table stores checked resolved destinations for every explicit depth and
the default depth. Preparation creates an immutable edge move plan for each
destination. Execution reads the selector as an unsigned i32, selects the
default when it is out of range, applies the simultaneous result moves, and
updates the logical label depth. A branch to the implicit function label
publishes its mapped results directly. Repeated destinations use the same
source values and label result types.

Move plans remove self moves and schedule writes after their old values are
no longer needed. Cycles use one typed scratch value. The production move
applicator is tested against a simultaneous-copy oracle for mixed-type swaps
and longer rotations; P130 retains its generated graph coverage.

The executor checks source ownership when creating an activation. An admitted
whole function has no calls or effects that could replace its module or
context during execution. Its private boundary lookup therefore reads the
immutable recipe directly instead of repeating the owner check at every
operation. Diagnostic materialization still checks ownership. A private test
moves prepared metadata onto a separately compiled module and proves that
activation declines it, including when the Engine and Wasm bytes match.

Every executed structural or numeric instruction still passes the common
fuel, cancellation and logical operand-height boundary. The register bank's
physical size does not change the logical stack limit. Source snapshots retain
typed operands, locals, labels, cursor and remaining fuel. Table tests compare
65 charged source prefixes with Decoded, and also with Compact when enabled.
A caller holding two live operands validates at stack limit three but makes
the callee reach logical height four before its table; fuel exhaustion takes
priority over the operand-limit error at the same boundary.

Table payloads, retained edge vectors, temporary lowering storage and move
planner work use checked arithmetic and the existing transactional work/byte
budgets. Unsupported or excessive whole functions decline before guest work.
Guest calls continue through Decoded callers and may enter admitted Register
callees. Preparation does not allocate during an executed branch.

The task's paired decision and durable evidence are recorded in
[P060-records.json](../benchmarks/performance/P060-records.json). Focused
candidate evidence does not replace the later portfolio gate.

The candidate gate rejected P060. Across 1,500 physical pairs, the equally
weighted target warm ratios were 0.79785 and 0.79852, with supported gains in
both runs. Small and held-out table load-and-call regressed, and large table
preparation exceeded its bound. The implementation remains behind the
nondefault feature because dependent float, memory and boundary work needs
its complete executable control interface. This result does not promote
Register or alter Decoded/Portable selection.
