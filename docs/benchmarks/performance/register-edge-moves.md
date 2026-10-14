# Bounded parallel edge moves

P130 prepares private immutable schedules for P058's simultaneous branch-result
move requests. It orders writes after the last read of each old slot value and
breaks cycles with one separate scratch value. Each scratch save preserves the
numeric type; disconnected and mixed-type cycles reuse the same scratch storage.

Types belong to values at a source boundary. A destination can reuse a temporary
height with another numeric type after its old value has been read. Conflicting
types for reads of the same old source value, duplicate destinations and invalid
slot indices decline. Checked work and peak-allocation bounds precede allocation;
failure leaves the shared work/byte budget unchanged. Publication charges retained
vector capacities and conservative preparation work.

Four private semantic cases cover empty/self moves, acyclic chains and fanout,
two/three-slot cycles, mixed types, the actual P058 shifted i32/i64 result edge,
wide indices, failed plans and shared budget exhaustion. A deterministic oracle
checks 4,096 generated move graphs: 18,858 scheduled steps and 2,401 scratch saves.
Work and retained-byte totals in the [proof](P130-proof.json) are sums across
separately prepared graphs, each with its own bounded budget.

The [records](P130-records.json) and [completion](P130-completion.json) bind all
14 focused/compatibility/default test executions and build identities. This is a
contract-only prerequisite. P061 consumes the scheduler in the initial integer
executor; P060 retains its complete control-transfer and performance gate.
Register policy remains unavailable, and Decoded remains the default.
