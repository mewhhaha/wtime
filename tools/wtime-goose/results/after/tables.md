### results/after/examples.json

3 rounds, CPU 1 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wtime-decoded run | wtime-before-decoded run | wtime-decoded / Wasmtime run | wtime-before-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.0 ms | 0.1 ms | 1.08× | 2.60× | match |
| tour | 0.1 ms | 1.1 ms | 10.1 ms | 11× | 101× | match |
| functions | 0.1 ms | 0.6 ms | 8.4 ms | 8.26× | 118× | match |
| errors | 0.0 ms | 0.6 ms | 8.1 ms | 13× | 185× | match |
| sieve | 0.6 ms | 172.8 ms | 723.6 ms | 253× | 1,018× | match |
| sorting | 151.9 ms | 97.1 s | 281.3 s | 639× | 1,871× | match |
| life | 28.9 ms | 5,035.3 ms | 16.5 s | 180× | 574× | match |
| maze | 0.1 ms | 4.6 ms | 23.5 ms | 65× | 334× | match |
| sudoku | 9.2 ms | 2,644.5 ms | 8,171.7 ms | 284× | 889× | match |
| vm | 0.1 ms | 2.8 ms | 14.8 ms | 37× | 187× | match |

| Case | Wasmtime prep | wtime-decoded prep | wtime-before-decoded prep | Wasmtime process | wtime-decoded process | wtime-before-decoded process |
|---|---:|---:|---:|---:|---:|---:|
| squares | 5.6 ms | 0.2 ms | 0.9 ms | 10.5 ms | 4.8 ms | 5.3 ms |
| tour | 53.2 ms | 2.3 ms | 3.8 ms | 59.6 ms | 8.3 ms | 19.5 ms |
| functions | 120.0 ms | 5.2 ms | 8.2 ms | 124.9 ms | 10.6 ms | 21.4 ms |
| errors | 32.2 ms | 1.3 ms | 2.5 ms | 36.6 ms | 6.0 ms | 14.9 ms |
| sieve | 25.0 ms | 0.6 ms | 1.6 ms | 33.0 ms | 177.9 ms | 732.4 ms |
| sorting | 117.6 ms | 7.5 ms | 12.5 ms | 274.0 ms | 97.2 s | 281.3 s |
| life | 23.6 ms | 1.1 ms | 1.8 ms | 57.9 ms | 5,041.4 ms | 16.5 s |
| maze | 47.6 ms | 1.8 ms | 3.1 ms | 53.0 ms | 11.5 ms | 32.1 ms |
| sudoku | 16.6 ms | 0.7 ms | 1.6 ms | 31.6 ms | 2,649.9 ms | 8,179.3 ms |
| vm | 38.5 ms | 1.6 ms | 2.9 ms | 46.0 ms | 9.7 ms | 24.5 ms |

### results/after/compiler.json

3 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-decoded run | wtime-before-decoded run | wasmtime-cached / Wasmtime run | wtime-decoded / Wasmtime run | wtime-before-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| compile-squares | 1.0 ms | 1.0 ms | 42.0 ms | 113.2 ms | 1.17× | 42× | 128× | match |
| compile-vm | 5.8 ms | 7.1 ms | 1,056.3 ms | 2,480.1 ms | 1.03× | 155× | 430× | match |
| compile-sudoku | 4.8 ms | 4.8 ms | 612.3 ms | 1,756.9 ms | 1.05× | 128× | 391× | match |
| compile-functions | 24.4 ms | 28.8 ms | 4,886.4 ms | 12.3 s | 1.11× | 200× | 476× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-decoded prep | wtime-before-decoded prep | Wasmtime process | wasmtime-cached process | wtime-decoded process | wtime-before-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 6,805.5 ms | 6.9 ms | 399.6 ms | 3,329.5 ms | 6,822.4 ms | 17.5 ms | 491.2 ms | 3,669.5 ms |
| compile-vm | 6,107.8 ms | 8.0 ms | 443.1 ms | 3,135.8 ms | 6,134.8 ms | 25.9 ms | 1,619.2 ms | 5,762.1 ms |
| compile-sudoku | 6,267.6 ms | 7.1 ms | 369.4 ms | 2,953.0 ms | 6,286.7 ms | 21.8 ms | 1,027.3 ms | 4,834.1 ms |
| compile-functions | 6,193.2 ms | 7.6 ms | 440.0 ms | 3,183.8 ms | 6,232.3 ms | 48.5 ms | 5,311.9 ms | 15.8 s |

### results/after/compiler64.json

3 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---|
| compile64-vm | 9.6 ms | 9.2 ms | 1,141.8 ms | 0.90× | 119× | match |
| compile64-functions | 36.8 ms | 38.3 ms | 5,413.5 ms | 1.04× | 142× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|
| compile64-vm | 10.9 s | 8.7 ms | 444.9 ms | 10.9 s | 28.5 ms | 1,660.2 ms |
| compile64-functions | 10.7 s | 9.1 ms | 447.1 ms | 10.7 s | 58.1 ms | 5,987.9 ms |

