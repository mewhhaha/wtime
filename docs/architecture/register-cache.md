# Bounded lazy Register preparation cache

The private `execution::register::cache` module provides a source-owned cache
for prepared functions. P133 validates this boundary and its resource limits;
the cache is not yet connected to `Engine`, `Store`, `ModuleData`, or guest
execution. Existing eager module preparation and execution policy remain
unchanged, and P133 makes no timing or performance claim.

An entry becomes eligible for preparation after 16 calls. The cache retains at
most 64 entries and bounds attempted preparation work at 100,000 units and
retained metadata at 1 MiB. Cache bookkeeping and retained `Arc` capacity are
included in the byte accounting. A single preparation lease may be active;
the cache releases its locks before invoking the bounded P132 function
preparer. Calls that cannot acquire bookkeeping promptly, encounter another
active lease, or do not belong to the owning engine and source decline without
waiting or publishing an artifact.

Failed preparation does not reset its retry history. A failed entry counts
down 128 further calls before it may retry, and it receives at most two
attempts. Cooldown and permanent negative entries remain indexed. At capacity,
FIFO eviction can remove only cold or ready entries, so eviction cannot erase
a failure's cooldown or retry cap. Attempted work is never refunded, including
work from declined or aborted leases. A poisoned ledger fails closed.

Published prepared artifacts retain their metadata charge through a retention
token. Evicting an entry does not refund those bytes while another `Arc` still
pins the artifact; the charge is released only after its final reference drops.
This keeps the byte limit meaningful across eviction and outstanding users.

P133's contract checks cover threshold publication and single building,
concurrent duplicate fallback without holding cache locks during building,
decline/poison/foreign-owner/aborted-lease behavior, and FIFO eviction,
pinned lifetime, and budget churn. They establish the standalone cache's
correctness and build-budget boundary, not integration or runtime speed.
