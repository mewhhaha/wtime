# Profile-guided compiler32 (`wtime-aot build --train`, 0c142ba)

The 32-bit Goose compiler (goose-to-wasm 45eb0c7, `build/cmp/compiler32.wasm`)
built twice by the same generator for the same CPU: plain, and with
`--train --allow-files / -- examples/sorting.goose --stdlib stdlib --target
wasm-direct -O2 -o /dev/null` (a training input none of the measured cases
uses). Both emit Wasm byte-identical to the native compiler's for squares,
vm, sudoku and functions.

Run time (`_start`) in ms, medians of 11 rounds, engines alternating order,
each a fresh process pinned to CPU 2; ratios are medians of per-round pairs.
Wasmtime runs from its warmed module cache.

| Case | plain | profile-guided | Wasmtime | pgo / plain | plain / Wasmtime | pgo / Wasmtime |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| compile-squares | 0.66 | 0.70 | 0.72 | 0.97× | 0.90× | 0.89× |
| compile-vm | 4.41 | 4.43 | 4.90 | 1.01× | 0.89× | 0.91× |
| compile-sudoku | 3.29 | 3.34 | 3.70 | 1.00× | 0.87× | 0.87× |
| compile-functions | 17.11 | 17.39 | 18.88 | 1.02× | 0.92× | 0.94× |

The profile changes nothing measurable here: the compiler's time is spread
over thousands of functions (the hottest takes 6%), with no hot path for the
profile to straighten. On sorting (one hot loop) it left the instruction
count unchanged (2,616.8 M either way). The option stays for programs whose
profiles are less flat; the benchmarks use plain builds.
