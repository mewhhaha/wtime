### results/aot-2b0e007/examples.json

5 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.0 ms | 0.0 ms | 0.3 ms | 0.0 ms | 0.91× | 0.12× | 14× | 1.07× | match |
| tour | 0.1 ms | 0.1 ms | 0.0 ms | 2.9 ms | 0.6 ms | 1.07× | 0.59× | 39× | 8.55× | match |
| functions | 0.1 ms | 0.1 ms | 0.0 ms | 3.0 ms | 0.6 ms | 1.17× | 0.60× | 46× | 8.43× | match |
| errors | 0.0 ms | 0.1 ms | 0.0 ms | 1.4 ms | 0.6 ms | 1.13× | 0.57× | 30× | 12× | match |
| sieve | 0.6 ms | 0.6 ms | 0.7 ms | 7.2 ms | 62.1 ms | 0.97× | 1.14× | 11× | 97× | match |
| sorting | 175.0 ms | 176.8 ms | 154.6 ms | 601.5 ms | 23.5 s | 1.02× | 0.83× | 3.24× | 134× | match |
| life | 21.8 ms | 22.5 ms | 14.3 ms | 26.1 ms | 1,863.9 ms | 1.02× | 0.63× | 1.20× | 83× | match |
| maze | 0.1 ms | 0.1 ms | 0.0 ms | 1.8 ms | 2.5 ms | 0.99× | 0.67× | 23× | 35× | match |
| sudoku | 9.3 ms | 11.4 ms | 9.2 ms | 15.7 ms | 1,038.1 ms | 0.99× | 1.00× | 1.61× | 109× | match |
| vm | 0.1 ms | 0.1 ms | 0.0 ms | 1.7 ms | 1.7 ms | 1.01× | 0.63× | 28× | 27× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 4.0 ms | 0.3 ms | 0.0 ms | 0.5 ms | 0.2 ms | 8.1 ms | 4.0 ms | 3.9 ms | 43.3 ms | 3.7 ms |
| tour | 36.1 ms | 0.4 ms | 0.0 ms | 0.6 ms | 1.0 ms | 40.5 ms | 4.6 ms | 4.0 ms | 46.4 ms | 5.9 ms |
| functions | 123.2 ms | 0.5 ms | 0.0 ms | 1.0 ms | 2.8 ms | 128.5 ms | 5.6 ms | 4.6 ms | 48.3 ms | 7.3 ms |
| errors | 32.4 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.9 ms | 37.4 ms | 5.0 ms | 4.2 ms | 45.7 ms | 5.4 ms |
| sieve | 11.2 ms | 0.3 ms | 0.0 ms | 0.6 ms | 0.4 ms | 16.2 ms | 5.0 ms | 4.7 ms | 50.8 ms | 67.2 ms |
| sorting | 108.1 ms | 0.6 ms | 0.0 ms | 0.9 ms | 3.1 ms | 288.5 ms | 182.2 ms | 159.8 ms | 651.1 ms | 23.5 s |
| life | 17.4 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.7 ms | 44.9 ms | 27.5 ms | 18.7 ms | 73.0 ms | 1,869.4 ms |
| maze | 37.0 ms | 0.4 ms | 0.0 ms | 0.7 ms | 1.1 ms | 41.9 ms | 4.9 ms | 3.9 ms | 48.5 ms | 8.0 ms |
| sudoku | 15.3 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.5 ms | 28.6 ms | 15.9 ms | 13.6 ms | 61.3 ms | 1,043.1 ms |
| vm | 28.6 ms | 0.4 ms | 0.0 ms | 0.7 ms | 0.8 ms | 33.2 ms | 4.2 ms | 3.8 ms | 47.1 ms | 6.5 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 0.3 ms | 0.0 ms | 0.0 ms | 0.01× | 0.09× | 0.5 ms | 0.0 ms | 0.2 ms | 43.3 ms | 3.9 ms | 3.7 ms | 0.09× | 0.09× |
| tour | 2.9 ms | 0.0 ms | 0.6 ms | 0.02× | 0.21× | 0.6 ms | 0.0 ms | 1.0 ms | 46.4 ms | 4.0 ms | 5.9 ms | 0.08× | 0.12× |
| functions | 3.0 ms | 0.0 ms | 0.6 ms | 0.01× | 0.20× | 1.0 ms | 0.0 ms | 2.8 ms | 48.3 ms | 4.6 ms | 7.3 ms | 0.08× | 0.15× |
| errors | 1.4 ms | 0.0 ms | 0.6 ms | 0.02× | 0.43× | 0.6 ms | 0.0 ms | 0.9 ms | 45.7 ms | 4.2 ms | 5.4 ms | 0.09× | 0.12× |
| sieve | 7.2 ms | 0.7 ms | 62.1 ms | 0.10× | 8.46× | 0.6 ms | 0.0 ms | 0.4 ms | 50.8 ms | 4.7 ms | 67.2 ms | 0.09× | 1.32× |
| sorting | 601.5 ms | 154.6 ms | 23.5 s | 0.26× | 40× | 0.9 ms | 0.0 ms | 3.1 ms | 651.1 ms | 159.8 ms | 23.5 s | 0.25× | 37× |
| life | 26.1 ms | 14.3 ms | 1,863.9 ms | 0.55× | 72× | 0.6 ms | 0.0 ms | 0.7 ms | 73.0 ms | 18.7 ms | 1,869.4 ms | 0.26× | 26× |
| maze | 1.8 ms | 0.0 ms | 2.5 ms | 0.03× | 1.42× | 0.7 ms | 0.0 ms | 1.1 ms | 48.5 ms | 3.9 ms | 8.0 ms | 0.08× | 0.16× |
| sudoku | 15.7 ms | 9.2 ms | 1,038.1 ms | 0.61× | 65× | 0.6 ms | 0.0 ms | 0.5 ms | 61.3 ms | 13.6 ms | 1,043.1 ms | 0.22× | 16× |
| vm | 1.7 ms | 0.0 ms | 1.7 ms | 0.02× | 0.92× | 0.7 ms | 0.0 ms | 0.8 ms | 47.1 ms | 3.8 ms | 6.5 ms | 0.08× | 0.14× |

### results/aot-2b0e007/compiler.json

5 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| compile-squares | 1.0 ms | 1.3 ms | 1.3 ms | 114.4 ms | 28.6 ms | 1.24× | 1.21× | 105× | 26× | match |
| compile-vm | 5.6 ms | 6.1 ms | 6.4 ms | 354.1 ms | 465.9 ms | 1.12× | 1.18× | 63× | 83× | match |
| compile-sudoku | 4.4 ms | 5.1 ms | 5.3 ms | 359.3 ms | 305.4 ms | 1.17× | 1.22× | 80× | 70× | match |
| compile-functions | 23.8 ms | 23.8 ms | 26.4 ms | 491.1 ms | 1,832.6 ms | 0.99× | 1.12× | 20× | 71× | match |
| compile64-vm | 8.2 ms | 9.3 ms | 14.1 ms | 1,129.5 ms | 593.2 ms | 1.13× | 1.71× | 137× | 72× | match |
| compile64-functions | 38.0 ms | 40.5 ms | 54.2 ms | 1,461.1 ms | 2,649.2 ms | 1.07× | 1.47× | 37× | 65× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 6,021.0 ms | 6.1 ms | 0.3 ms | 47.3 ms | 207.7 ms | 6,036.4 ms | 16.7 ms | 5.8 ms | 258.5 ms | 254.2 ms |
| compile-vm | 5,980.5 ms | 6.1 ms | 0.2 ms | 45.8 ms | 218.5 ms | 6,002.4 ms | 21.3 ms | 10.8 ms | 507.2 ms | 709.2 ms |
| compile-sudoku | 5,666.4 ms | 6.1 ms | 0.3 ms | 46.5 ms | 192.6 ms | 5,686.7 ms | 20.6 ms | 10.5 ms | 543.4 ms | 514.2 ms |
| compile-functions | 5,545.0 ms | 6.0 ms | 0.3 ms | 45.3 ms | 191.7 ms | 5,585.3 ms | 38.5 ms | 31.8 ms | 647.7 ms | 2,041.2 ms |
| compile64-vm | 9,557.9 ms | 6.9 ms | 0.3 ms | 53.5 ms | 237.6 ms | 9,586.0 ms | 26.7 ms | 19.0 ms | 1,298.2 ms | 852.2 ms |
| compile64-functions | 9,557.5 ms | 7.3 ms | 0.3 ms | 55.4 ms | 240.8 ms | 9,633.5 ms | 58.0 ms | 60.1 ms | 1,663.9 ms | 2,909.7 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 114.4 ms | 1.3 ms | 28.6 ms | 0.01× | 0.26× | 47.3 ms | 0.3 ms | 207.7 ms | 258.5 ms | 5.8 ms | 254.2 ms | 0.02× | 0.94× |
| compile-vm | 354.1 ms | 6.4 ms | 465.9 ms | 0.02× | 1.32× | 45.8 ms | 0.2 ms | 218.5 ms | 507.2 ms | 10.8 ms | 709.2 ms | 0.02× | 1.41× |
| compile-sudoku | 359.3 ms | 5.3 ms | 305.4 ms | 0.01× | 0.85× | 46.5 ms | 0.3 ms | 192.6 ms | 543.4 ms | 10.5 ms | 514.2 ms | 0.02× | 1.01× |
| compile-functions | 491.1 ms | 26.4 ms | 1,832.6 ms | 0.05× | 3.61× | 45.3 ms | 0.3 ms | 191.7 ms | 647.7 ms | 31.8 ms | 2,041.2 ms | 0.05× | 3.13× |
| compile64-vm | 1,129.5 ms | 14.1 ms | 593.2 ms | 0.01× | 0.52× | 53.5 ms | 0.3 ms | 237.6 ms | 1,298.2 ms | 19.0 ms | 852.2 ms | 0.01× | 0.65× |
| compile64-functions | 1,461.1 ms | 54.2 ms | 2,649.2 ms | 0.03× | 1.75× | 55.4 ms | 0.3 ms | 240.8 ms | 1,663.9 ms | 60.1 ms | 2,909.7 ms | 0.03× | 1.71× |

