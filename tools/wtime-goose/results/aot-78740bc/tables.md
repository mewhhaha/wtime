### results/aot-78740bc/examples.json

5 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.0 ms | 0.0 ms | 0.3 ms | 0.0 ms | 0.75× | 0.10× | 14× | 1.76× | match |
| tour | 0.1 ms | 0.1 ms | 0.1 ms | 3.0 ms | 0.7 ms | 1.00× | 0.91× | 41× | 8.88× | match |
| functions | 0.1 ms | 0.1 ms | 0.0 ms | 2.9 ms | 0.6 ms | 1.18× | 0.61× | 42× | 8.72× | match |
| errors | 0.0 ms | 0.0 ms | 0.0 ms | 1.4 ms | 0.7 ms | 1.02× | 0.53× | 31× | 16× | match |
| sieve | 0.7 ms | 0.7 ms | 0.6 ms | 7.3 ms | 62.4 ms | 0.89× | 0.93× | 9.69× | 91× | match |
| sorting | 141.5 ms | 139.5 ms | 112.8 ms | 321.4 ms | 22.4 s | 1.01× | 0.82× | 2.27× | 160× | match |
| life | 23.9 ms | 24.2 ms | 5.4 ms | 25.3 ms | 1,847.1 ms | 1.00× | 0.24× | 1.15× | 79× | match |
| maze | 0.1 ms | 0.1 ms | 0.0 ms | 1.9 ms | 2.5 ms | 0.96× | 0.58× | 27× | 35× | match |
| sudoku | 9.1 ms | 9.0 ms | 6.7 ms | 16.6 ms | 1,000.2 ms | 0.95× | 0.74× | 1.81× | 106× | match |
| vm | 0.1 ms | 0.1 ms | 0.0 ms | 1.8 ms | 1.6 ms | 1.12× | 0.61× | 31× | 28× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 4.2 ms | 0.3 ms | 0.0 ms | 0.6 ms | 0.2 ms | 8.6 ms | 4.8 ms | 4.1 ms | 45.9 ms | 4.4 ms |
| tour | 38.0 ms | 0.4 ms | 0.0 ms | 0.8 ms | 1.1 ms | 42.9 ms | 5.0 ms | 4.2 ms | 51.6 ms | 6.0 ms |
| functions | 124.3 ms | 0.5 ms | 0.0 ms | 1.0 ms | 3.1 ms | 130.1 ms | 5.3 ms | 4.2 ms | 51.0 ms | 8.2 ms |
| errors | 34.2 ms | 0.4 ms | 0.0 ms | 0.7 ms | 1.0 ms | 39.0 ms | 5.2 ms | 4.3 ms | 50.9 ms | 5.9 ms |
| sieve | 11.8 ms | 0.4 ms | 0.0 ms | 0.7 ms | 0.4 ms | 17.3 ms | 5.9 ms | 5.1 ms | 56.7 ms | 67.9 ms |
| sorting | 107.4 ms | 0.6 ms | 0.0 ms | 1.3 ms | 3.6 ms | 250.8 ms | 145.7 ms | 117.3 ms | 381.6 ms | 22.4 s |
| life | 18.1 ms | 0.5 ms | 0.0 ms | 0.6 ms | 0.7 ms | 47.2 ms | 30.2 ms | 9.9 ms | 74.4 ms | 1,852.6 ms |
| maze | 40.4 ms | 0.4 ms | 0.0 ms | 0.8 ms | 1.0 ms | 45.6 ms | 5.1 ms | 4.4 ms | 51.8 ms | 7.7 ms |
| sudoku | 16.8 ms | 0.4 ms | 0.0 ms | 0.7 ms | 0.5 ms | 30.8 ms | 14.3 ms | 11.8 ms | 66.2 ms | 1,006.1 ms |
| vm | 27.8 ms | 0.4 ms | 0.0 ms | 0.7 ms | 0.8 ms | 32.1 ms | 5.1 ms | 4.2 ms | 47.1 ms | 6.4 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 0.3 ms | 0.0 ms | 0.0 ms | 0.01× | 0.13× | 0.6 ms | 0.0 ms | 0.2 ms | 45.9 ms | 4.1 ms | 4.4 ms | 0.09× | 0.09× |
| tour | 3.0 ms | 0.1 ms | 0.7 ms | 0.02× | 0.22× | 0.8 ms | 0.0 ms | 1.1 ms | 51.6 ms | 4.2 ms | 6.0 ms | 0.08× | 0.12× |
| functions | 2.9 ms | 0.0 ms | 0.6 ms | 0.01× | 0.21× | 1.0 ms | 0.0 ms | 3.1 ms | 51.0 ms | 4.2 ms | 8.2 ms | 0.08× | 0.16× |
| errors | 1.4 ms | 0.0 ms | 0.7 ms | 0.02× | 0.48× | 0.7 ms | 0.0 ms | 1.0 ms | 50.9 ms | 4.3 ms | 5.9 ms | 0.09× | 0.12× |
| sieve | 7.3 ms | 0.6 ms | 62.4 ms | 0.09× | 8.59× | 0.7 ms | 0.0 ms | 0.4 ms | 56.7 ms | 5.1 ms | 67.9 ms | 0.09× | 1.20× |
| sorting | 321.4 ms | 112.8 ms | 22.4 s | 0.36× | 71× | 1.3 ms | 0.0 ms | 3.6 ms | 381.6 ms | 117.3 ms | 22.4 s | 0.32× | 60× |
| life | 25.3 ms | 5.4 ms | 1,847.1 ms | 0.24× | 71× | 0.6 ms | 0.0 ms | 0.7 ms | 74.4 ms | 9.9 ms | 1,852.6 ms | 0.15× | 24× |
| maze | 1.9 ms | 0.0 ms | 2.5 ms | 0.02× | 1.36× | 0.8 ms | 0.0 ms | 1.0 ms | 51.8 ms | 4.4 ms | 7.7 ms | 0.08× | 0.16× |
| sudoku | 16.6 ms | 6.7 ms | 1,000.2 ms | 0.40× | 61× | 0.7 ms | 0.0 ms | 0.5 ms | 66.2 ms | 11.8 ms | 1,006.1 ms | 0.17× | 15× |
| vm | 1.8 ms | 0.0 ms | 1.6 ms | 0.02× | 0.90× | 0.7 ms | 0.0 ms | 0.8 ms | 47.1 ms | 4.2 ms | 6.4 ms | 0.09× | 0.13× |

### results/aot-78740bc/compiler.json

5 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| compile-squares | 1.1 ms | 1.3 ms | 1.4 ms | 100.0 ms | 26.1 ms | 1.19× | 1.23× | 91× | 23× | match |
| compile-vm | 6.1 ms | 6.0 ms | 5.7 ms | 348.3 ms | 403.9 ms | 0.98× | 0.91× | 59× | 68× | match |
| compile-sudoku | 4.7 ms | 4.9 ms | 4.5 ms | 352.0 ms | 293.7 ms | 1.06× | 0.94× | 73× | 62× | match |
| compile-functions | 24.7 ms | 25.1 ms | 22.3 ms | 529.4 ms | 1,761.4 ms | 1.01× | 0.88× | 21× | 69× | match |
| compile64-vm | 8.9 ms | 9.8 ms | 11.9 ms | 1,187.9 ms | 598.2 ms | 1.08× | 1.34× | 128× | 69× | match |
| compile64-functions | 36.8 ms | 36.4 ms | 42.7 ms | 1,475.1 ms | 2,584.9 ms | 1.04× | 1.16× | 41× | 70× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 5,536.0 ms | 6.6 ms | 0.2 ms | 47.9 ms | 191.0 ms | 5,552.6 ms | 17.0 ms | 6.2 ms | 244.9 ms | 233.1 ms |
| compile-vm | 5,497.1 ms | 6.5 ms | 0.3 ms | 46.6 ms | 190.9 ms | 5,519.2 ms | 21.4 ms | 10.4 ms | 508.3 ms | 610.8 ms |
| compile-sudoku | 5,736.2 ms | 6.9 ms | 0.2 ms | 48.0 ms | 190.8 ms | 5,758.4 ms | 21.6 ms | 9.6 ms | 522.5 ms | 498.7 ms |
| compile-functions | 5,551.4 ms | 7.2 ms | 0.3 ms | 48.2 ms | 191.9 ms | 5,592.8 ms | 41.7 ms | 27.3 ms | 691.3 ms | 1,972.9 ms |
| compile64-vm | 9,997.7 ms | 8.6 ms | 0.3 ms | 54.0 ms | 240.2 ms | 10.0 s | 29.5 ms | 18.1 ms | 1,355.5 ms | 856.7 ms |
| compile64-functions | 9,825.4 ms | 8.7 ms | 0.3 ms | 54.6 ms | 234.9 ms | 9,893.4 ms | 56.1 ms | 48.4 ms | 1,645.6 ms | 2,860.7 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 100.0 ms | 1.4 ms | 26.1 ms | 0.01× | 0.26× | 47.9 ms | 0.2 ms | 191.0 ms | 244.9 ms | 6.2 ms | 233.1 ms | 0.03× | 0.93× |
| compile-vm | 348.3 ms | 5.7 ms | 403.9 ms | 0.02× | 1.16× | 46.6 ms | 0.3 ms | 190.9 ms | 508.3 ms | 10.4 ms | 610.8 ms | 0.02× | 1.20× |
| compile-sudoku | 352.0 ms | 4.5 ms | 293.7 ms | 0.01× | 0.82× | 48.0 ms | 0.2 ms | 190.8 ms | 522.5 ms | 9.6 ms | 498.7 ms | 0.02× | 0.99× |
| compile-functions | 529.4 ms | 22.3 ms | 1,761.4 ms | 0.04× | 3.46× | 48.2 ms | 0.3 ms | 191.9 ms | 691.3 ms | 27.3 ms | 1,972.9 ms | 0.04× | 2.97× |
| compile64-vm | 1,187.9 ms | 11.9 ms | 598.2 ms | 0.01× | 0.49× | 54.0 ms | 0.3 ms | 240.2 ms | 1,355.5 ms | 18.1 ms | 856.7 ms | 0.01× | 0.61× |
| compile64-functions | 1,475.1 ms | 42.7 ms | 2,584.9 ms | 0.03× | 1.73× | 54.6 ms | 0.3 ms | 234.9 ms | 1,645.6 ms | 48.4 ms | 2,860.7 ms | 0.03× | 1.69× |

