# P040 original control fixture packet — manual oracle, not execution evidence

These six WAT sources were written for the compact-control proposal and licensed under the included MIT license. They are deliberately source-only. No WAT parser, encoder, validator, wasm runtime, Rust code, Cargo command, or performance command has been run. The module/function validity and all outcomes below are hand-derived from the WAT text; treat them as expected cases to validate before integration, not as validated Wasm or passing tests.

Source event convention: each original Wasm instruction is one event, including `block`, `loop`, `if`, executed `else`, `end`, branches, `return`, and `unreachable`; the final function `end` is an event when normal sequential execution reaches it. An instruction skipped by a branch/conditional jump is not an event. Fuel is checked/charged before executing the next source instruction. A branch to the function label or explicit `return` completes the function directly; it does not execute the lexical final function `end`. This follows P020's exact source-boundary contract and is the expected oracle convention to verify against decoded execution.

## Cases and expected paths

`multivalue_loop.wat`, export `run`, no arguments, expected results `(i32 2, i64 40)`. The `i32/i64` pair passes unchanged through a block and loop with matching parameter/result arity. Local `$i` starts at zero and the loop exits when it reaches two. Executed source PCs: `0..12, 4..12, 4..7, 15` (27 events; both loop `end` pc13 and branch-target block `end` pc14 are skipped). Full-fuel expected count: 27. Fuel limit 26 must stop before pc15 with pair `(2,40)` materialized and one unit unpaid for the next source event; with limit 27 it returns normally. This covers loop-carried values, taken/untaken `br_if`, loop backedges, and block/function ends.

`nested_if_results.wat`, export `run`, expected `(i32,i64)`:

- `(outer=0, inner=0)` -> `(-1,-2)`, PCs `0,1,11,12,13,14`, 6 events. Outer false skips else marker pc10 and the entire then arm.
- `(1,0)` -> `(42,421)`, PCs `0,1,2,3,7,8,9,10,13,14`, 10 events. Inner false skips marker pc6; outer true reaches pc10, whose else action skips the outer else arm.
- `(1,1)` -> `(41,420)`, PCs `0,1,2,3,4,5,6,9,10,13,14`, 11 events; both taken then arms reach their executed else markers.

The fuel-stop prefix cases should use the listed exact paths; e.g. fuel 4 in `(1,0)` fails before pc7 with nested false-branch state intact, fuel 9 fails before pc14 after executing outer else, and fuel 10 completes. Confirm the decoded interpreter's precise conditional-skip behavior before treating these counts as certified.

`br_if_taken_untaken.wat`, `run(1)` -> `42`, path PCs `0,1,2,3`, 4 events (branch-to-function); `run(0)` -> `7`, PCs `0..7`, 8 events. Untaken branch consumes its condition but preserves block result 42 until `drop`; taken branch preserves that result to the function label and skips the fallthrough and both end instructions. For `run(0)`, fuel 7 stops before function end pc7 after the block result is materialized; fuel 8 completes.

`br_table_three_paths.wat`, expected result and path by `$which`:

- `0` selects explicit target depth 0: `1107`, PCs `0,1,2,3,4,6,7,8,9,10,11`, 11 events. Branching to inner block depth 0 resumes after its end at pc5; the outer block end pc8 is then reached normally.
- `1` selects explicit target depth 1: `107`, PCs `0,1,2,3,4,9,10,11`, 8 events. Branching to the outer block resumes after its end at pc8.
- `2` or any larger/default index selects depth 2/function label: `7`, PCs `0,1,2,3,4`, 5 events; function end pc11 is not visited.

The branch table's three labels all carry one i32, so this checks distinct destinations without mismatched label arities. Fuel 4 stops before executing `br_table` at pc4 for every input, preserving operand stack `[7, which]` and cursor at pc4. This is the common pre-branch prefix; the three full paths establish distinct depth-0, depth-1, and default/function destinations.

`function_branch_vs_return.wat`:

- `branch(1)` -> 42, PCs `0,1,2,3`, 4 events; `branch(0)` -> 7, PCs `0..7`, 8 events.
- `return(1)` -> 42, PCs `0,1,2,3`, 4 events; `return(0)` -> 7, PCs `0,1,5,6,7`, 5 events.

Both early exits stop before later lexical source events while preserving the same public result. This distinguishes branch-to-function from explicit return and checks their return paths share result publication but keep source origins.

`unreachable_local_prefix.wat`: fuel limit 2 stops before executing pc2 with visible local `$visible=9` and cursor at pc2; fuel limit 3 charges pc2 then traps as unreachable, with no execution of pc3/4. With unmetered execution the same trap occurs after the same three source events. The compact test should inspect materialized locals/cursor at the pre-pc2 fuel boundary; a public trap result alone does not prove the state prefix.

## Deferred coverage

These originals use only i32/i64 values. A V128/f32/f64 structured-control fixture is deferred until P039's adapters exist; these tests do not prove vector-valued branch/result moves. Tests must validate the WAT sources and obtain outcomes/fuel/cursor snapshots from the decoded oracle before serving as differential assertions. Keep the hand-derived expectations and report any discrepancy rather than silently updating them to match the candidate.
