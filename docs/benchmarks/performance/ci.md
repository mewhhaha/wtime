# Performance verification lanes

Ordinary CI runs the portable correctness smoke on x86_64 and AArch64. It builds
the default CLI, records its source and executable hashes, and invokes three
fixed MVP modules: `call_overhead`, `integer_loop`, and `branch_calls`. Every
result must match the separately bound P001 oracle. Zero work, changed fixture
bytes, a stale executable binding, or unexpected output fails the job. These
checks make no timing claim. GitHub retains their reports for 14 days.

```sh
cargo build --locked --offline -j 1 -p wtime-cli
python3 -B tools/verification/producers/p111.py bind \
  --binary target/debug/wtime-cli --output target/performance/P111/build.json
python3 -B tools/checks/performance_ci.py --lane smoke \
  --output target/performance/P111/smoke.json
```

The existing conformance jobs check normative behavior and the dependency gate
checks the default production closure. The P109 release contract binds the full
selected-tier normative matrix, source-event and interaction tests, deterministic
fuzz/vector checks and the process-isolated allocation failure. The P110 release
contract separately enforces build, memory, size and preparation budgets.
Integrity, semantics and absolute resource limits remain failures on every lane.

A shared worker may display a previously finalized paired measurement as a trend:

```sh
python3 -B tools/checks/performance_ci.py --lane trend \
  --shared-statistics PATH/TO/statistics.json \
  --completion PATH/TO/completion.json --output NEW/trend.json
```

This revalidates current inputs, command logs, named tests, immutable artifacts,
the two-run/30-pair protocol and the finalized decision. It does not bootstrap
again or time any Wasm. The trend decision is always `inconclusive`, including
when its retained measurement recorded `adopted`. Development, synthetic,
incomplete, superseded and stale evidence cannot pass this reader.

Relative regression and promotion decisions require an explicitly provisioned
dedicated runner. There is no such runner configured for this repository yet;
`--lane reference` without its inputs writes `unrun`. A scheduled operator job
must use the same source-bound measurement contract as release verification,
with the immutable predecessor archive, pinned comparator, toolchain, fixtures,
raw paired observations and all semantic/build gates. It runs
`tools/verify.py close --contract CONTRACT --output NEW/closure --execute` once.
It must not build the baseline from the candidate checkout.

The weekly/manual `performance-reference.yml` workflow emits that explicit
`unrun` report until repository variable `WTIME_REFERENCE_ENABLED` is `true`.
Enabling it requires a self-hosted `linux`, `x64`, `wtime-reference` runner with
the pinned tools, immutable baselines and retained verification store provisioned.
Set `WTIME_REFERENCE_RUNNER`, `WTIME_REFERENCE_CONTRACT`,
`WTIME_REFERENCE_CONFIG` to its declared ID and repository-contained paths, and
`WTIME_REFERENCE_STATISTICS_ARTIFACT` to the statistics output name in that
contract's analyze phase. The reader resolves its immutable path from the new
completion record. The workflow serializes runs and preserves failure
artifacts; enabling it does not bypass any full-contract checks.

Before that measurement, create a repository-contained reference configuration:

```json
{
  "schema": "wtime-p111-reference-v1",
  "runner_id": "operator-assigned-dedicated-runner-id",
  "architecture": "x86_64",
  "exclusive_runner": true,
  "hardware": {
    "cpu_model": "exact model reported by the collector",
    "selected_cpu": 0,
    "cpu_quota": "exact /sys/fs/cgroup/cpu.max contents without trailing newline"
  }
}
```

Add this file as an identity consumed by the contract's `measure` phase. Add
`WTIME_REFERENCE_RUNNER` to that phase's `environment` list and set it to the
same runner ID for collection. The operator is responsible for exclusive
machine provisioning; CPU affinity alone does not establish exclusivity. The
reader requires the pre-measurement config hash, recorded runner environment
hash and collected hardware to match. A declaration added after timing cannot
turn a shared-worker run into a reference run.

```sh
python3 -B tools/checks/performance_ci.py --lane reference \
  --reference-config PATH/TO/reference.json \
  --shared-statistics PATH/TO/statistics.json \
  --completion PATH/TO/completion.json --output NEW/reference.json
```

Only this lane carries the already computed `adopted`, `rejected` or
`inconclusive` decision. Missing infrastructure or unsupported architectures are
`unrun`, not successful zero-cost comparisons. The initial reader accepts the
shared verifier's single-comparison statistics schema; portfolio reports need
their own adapter before use. Retain release raw pairs, source snapshots and
proofs permanently; retain scheduled diagnostic reports for 30 days. Retention
must never delete an artifact referenced by a published release proof. Wasmtime
and any future native/AOT tools stay outside the default Rust dependency closure.
