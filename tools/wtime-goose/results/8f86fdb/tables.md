### results/8f86fdb/examples.json

3 rounds, CPU 1 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wtime-decoded run | wtime-before-decoded run | wtime-decoded / Wasmtime run | wtime-before-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.0 ms | 0.0 ms | 0.79× | 2.88× | match |
| tour | 0.1 ms | 0.5 ms | 9.9 ms | 8.39× | 147× | match |
| functions | 0.1 ms | 0.5 ms | 8.9 ms | 8.14× | 151× | match |
| errors | 0.0 ms | 0.4 ms | 7.8 ms | 10× | 203× | match |
| sieve | 0.6 ms | 83.1 ms | 597.9 ms | 139× | 1,043× | match |
| sorting | 174.5 ms | 38.6 s | 277.1 s | 221× | 1,581× | match |
| life | 22.1 ms | 2,437.2 ms | 16.2 s | 110× | 736× | match |
| maze | 0.1 ms | 2.6 ms | 24.7 ms | 37× | 327× | match |
| sudoku | 10.1 ms | 1,249.2 ms | 7,850.2 ms | 126× | 779× | match |
| vm | 0.1 ms | 1.5 ms | 11.7 ms | 26× | 211× | match |

| Case | Wasmtime prep | wtime-decoded prep | wtime-before-decoded prep | Wasmtime process | wtime-decoded process | wtime-before-decoded process |
|---|---:|---:|---:|---:|---:|---:|
| squares | 3.7 ms | 0.1 ms | 0.6 ms | 12.0 ms | 5.1 ms | 3.8 ms |
| tour | 44.6 ms | 1.4 ms | 3.5 ms | 48.8 ms | 5.8 ms | 18.7 ms |
| functions | 125.4 ms | 3.8 ms | 7.7 ms | 129.0 ms | 7.6 ms | 20.3 ms |
| errors | 30.9 ms | 1.1 ms | 2.2 ms | 34.1 ms | 4.3 ms | 13.4 ms |
| sieve | 11.1 ms | 0.4 ms | 1.2 ms | 14.8 ms | 86.3 ms | 602.9 ms |
| sorting | 107.3 ms | 4.9 ms | 9.1 ms | 298.8 ms | 38.6 s | 277.2 s |
| life | 17.4 ms | 0.7 ms | 1.8 ms | 43.6 ms | 2,442.9 ms | 16.2 s |
| maze | 41.0 ms | 1.5 ms | 3.2 ms | 46.2 ms | 8.1 ms | 32.7 ms |
| sudoku | 16.0 ms | 0.6 ms | 1.5 ms | 30.8 ms | 1,254.6 ms | 7,857.7 ms |
| vm | 27.8 ms | 0.9 ms | 2.1 ms | 32.0 ms | 6.1 ms | 19.4 ms |

### results/8f86fdb/compiler.json

3 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---|
| compile-squares | 0.9 ms | 1.1 ms | 22.6 ms | 1.31× | 26× | match |
| compile-vm | 5.5 ms | 5.7 ms | 461.2 ms | 1.04× | 89× | match |
| compile-sudoku | 4.4 ms | 4.6 ms | 323.9 ms | 1.05× | 74× | match |
| compile-functions | 21.2 ms | 22.7 ms | 2,245.3 ms | 1.07× | 103× | match |
| compile64-vm | 8.4 ms | 8.9 ms | 616.9 ms | 1.06× | 74× | match |
| compile64-functions | 32.6 ms | 33.1 ms | 2,964.3 ms | 1.02× | 89× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|
| compile-squares | 5,278.7 ms | 5.5 ms | 292.2 ms | 5,293.0 ms | 13.8 ms | 333.7 ms |
| compile-vm | 5,200.8 ms | 5.6 ms | 294.1 ms | 5,218.9 ms | 18.7 ms | 768.2 ms |
| compile-sudoku | 5,185.2 ms | 5.5 ms | 308.3 ms | 5,203.0 ms | 17.6 ms | 648.5 ms |
| compile-functions | 5,234.6 ms | 5.7 ms | 296.7 ms | 5,268.6 ms | 36.8 ms | 2,597.7 ms |
| compile64-vm | 9,231.6 ms | 7.4 ms | 348.5 ms | 9,258.5 ms | 26.2 ms | 988.9 ms |
| compile64-functions | 9,133.7 ms | 6.6 ms | 354.7 ms | 9,187.0 ms | 49.3 ms | 3,338.2 ms |

