# Strict scalar float register execution

P062 adds scalar f32/f64 execution to the nondefault Register feature. The
existing typed lowering and raw u64 bank already represent all four numeric
types. Constants, local moves, arguments, results and diagnostic snapshots
now admit float bits without changing their type or NaN payload. SIMD and
reference-bearing whole functions still decline before guest work.

The private float helper implements 70 scalar operations: comparisons,
arithmetic, rounding, trapping and saturating integer conversions, integer to
float conversions, demotion/promotion and reinterpretation. It uses safe
Rust's `from_bits`/`to_bits` and precise arithmetic. It allocates no operand
stack. There is no approximate arithmetic or fast-math setting.

Abs, negation and copysign retain their required payload bits. Arithmetic
unary operations quiet signaling NaNs. Min/max preserve signed-zero ordering
and allowed arithmetic NaNs. Trapping conversion truncates first, rejects
NaN as `invalid integer conversion`, and rejects the exclusive upper bound
as `integer overflow`. Saturating conversion maps NaN to zero and clamps the
endpoints. i32 and f32 slots retain zero-extended 32-bit patterns.

Preparation uses the existing whole-function work and metadata budgets.
Activation checks every argument/local type before using its bit bank. The
source-boundary path still charges every original instruction and checks
cancellation and logical operand height. Materialization reconstructs float
locals and operands with their exact declared types.

The task compares 15,966 boundary cases against Decoded across all 70
operations, proving 62,893 Register source events and no fallback frames.
Independent expected-bit cases cover signed zero, raw payloads, copysign and
allowed quiet NaNs. Separate conversion tests cover fuel/trap priority and
saturating endpoints. All 164 source prefixes of mixed locals and two actual
f32/f64 kernel loops compare full snapshots with Decoded, and with Compact
when enabled. The fixture encoder, independent decoder and stable-core
validator each run before those fixtures become runtime evidence.

The paired decision and durable records are in
[P062-records.json](../benchmarks/performance/P062-records.json). A focused
candidate result does not promote Register to the default policy or replace
the later portfolio gate.

The gate rejected P062 despite supported target warm-call ratios of 0.75216
and 0.75720 in the two runs. Small and zero-iteration kernel preparation and
cold calls regressed. The scalar numeric interface remains opt-in for the
dependent memory and boundary tasks; Decoded and Portable remain unchanged.
