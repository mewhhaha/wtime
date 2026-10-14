# Bounded prefixes before decoded handoff

P131 supplies a private prefix constructor for the later P064 integration.
Whole-function verification still requires complete source coverage and closed
control labels. The separate prefix constructor retains original source
events through a pending barrier, permits open block/loop labels, and requires
an `ExitToDecoded` metadata marker at that barrier. Production preparation
still selects whole functions only. P131 does not execute a decoded handoff.

The first subset runs numeric operations, local moves, constants, drop, nop,
block/loop/end and supported unshared memory accesses in source order. The
first conditional or nonlocal control transfer, call, return, reference/GC or
exception operation, growth, atomic or unsupported memory form is a barrier.
This keeps call-kernel block/loop setup and condition computation representable
while leaving the original `br_if` pending. Empty prefixes decline.

The barrier recipe carries the original body and PC, typed locals and ordered
operands. Existing source control metadata supplies the logical labels.
The source byte owner, Engine, body and layout checks remain intact. The
prefix verifier independently propagates the numeric stack shape and compares
every recipe, including the final barrier shape. It rejects event holes,
changed cursors, types, fuel metadata, invalid slots or ordinary whole-function
publication of a partial draft.

The scan, lowering and verification share transactional 100,000-work and
1-MiB metadata limits. Temporary stack-shape verification storage is included
in the allocation peak; actual retained capacities are charged before
publication. Failures leave the caller's budgets unchanged. Source bytes are
retained without creating a ModuleData ownership cycle.

The private model compares 52 typed source boundaries against fresh Decoded
snapshots. It checks complete locals, operands and labels, plus separate full
memory images before each instruction. The model executes no barrier and
charges only preceding original operations. The recorded subset marker
covers 26 boundaries, open label depth two and two memory prefixes, with
maximum retained metadata 2,179 bytes and charged work 259. Negative cases
cover foreign source owners, invalid barrier/cursor/type/slot/fuel metadata,
missing events, strict whole-function rejection, malformed control metadata,
empty prefixes and exhausted budgets.

P064 must materialize the real production bank and preserve the same frame,
outer frames, prompts, context and already charged fuel. It then executes
the pending opcode once through the normal Decoded path in that iteration.
The foundation does not authorize restarting an invocation or replaying an
earlier store.

Immutable correctness and build evidence is indexed by
[P131-records.json](../benchmarks/performance/P131-records.json). There is no
benchmark or speed decision for this foundation.
