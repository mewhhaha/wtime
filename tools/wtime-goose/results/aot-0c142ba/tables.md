### results/aot-0c142ba/examples.json

5 rounds, CPU 1 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.0 ms | 0.0 ms | 0.3 ms | 0.0 ms | 0.88× | 0.11× | 16× | 1.61× | match |
| tour | 0.1 ms | 0.1 ms | 0.1 ms | 3.5 ms | 0.7 ms | 1.03× | 0.64× | 40× | 8.37× | match |
| functions | 0.1 ms | 0.1 ms | 0.0 ms | 2.9 ms | 0.6 ms | 1.23× | 0.65× | 43× | 9.11× | match |
| errors | 0.0 ms | 0.0 ms | 0.0 ms | 1.3 ms | 0.6 ms | 1.13× | 0.54× | 30× | 13× | match |
| sieve | 0.7 ms | 0.7 ms | 0.6 ms | 7.4 ms | 61.5 ms | 0.98× | 0.81× | 9.97× | 90× | match |
| sorting | 195.6 ms | 209.1 ms | 132.7 ms | 637.5 ms | 24.8 s | 1.05× | 0.69× | 3.30× | 127× | match |
| life | 22.9 ms | 23.4 ms | 5.2 ms | 26.8 ms | 1,874.1 ms | 1.00× | 0.23× | 1.16× | 84× | match |
| maze | 0.1 ms | 0.1 ms | 0.0 ms | 1.9 ms | 2.6 ms | 1.17× | 0.63× | 27× | 37× | match |
| sudoku | 9.7 ms | 9.9 ms | 8.0 ms | 15.2 ms | 1,050.2 ms | 1.02× | 0.81× | 1.61× | 106× | match |
| vm | 0.1 ms | 0.1 ms | 0.0 ms | 1.9 ms | 1.9 ms | 1.00× | 0.64× | 28× | 30× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 4.2 ms | 0.3 ms | 0.0 ms | 0.5 ms | 0.2 ms | 8.8 ms | 4.4 ms | 3.7 ms | 45.5 ms | 4.1 ms |
| tour | 40.9 ms | 0.4 ms | 0.0 ms | 0.7 ms | 1.1 ms | 46.1 ms | 4.9 ms | 3.9 ms | 56.2 ms | 5.7 ms |
| functions | 126.4 ms | 0.5 ms | 0.0 ms | 0.9 ms | 3.1 ms | 130.7 ms | 4.6 ms | 3.7 ms | 48.3 ms | 7.5 ms |
| errors | 33.2 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.9 ms | 37.3 ms | 4.3 ms | 3.5 ms | 42.7 ms | 4.8 ms |
| sieve | 12.2 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.4 ms | 17.1 ms | 4.9 ms | 4.3 ms | 52.4 ms | 65.6 ms |
| sorting | 120.1 ms | 0.5 ms | 0.0 ms | 1.0 ms | 3.5 ms | 322.4 ms | 214.9 ms | 137.4 ms | 686.2 ms | 24.9 s |
| life | 18.4 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.6 ms | 44.8 ms | 28.0 ms | 9.3 ms | 75.2 ms | 1,879.5 ms |
| maze | 40.6 ms | 0.4 ms | 0.0 ms | 0.7 ms | 1.1 ms | 44.7 ms | 4.5 ms | 3.7 ms | 47.1 ms | 7.3 ms |
| sudoku | 17.3 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.5 ms | 31.2 ms | 14.2 ms | 11.6 ms | 62.4 ms | 1,055.0 ms |
| vm | 30.5 ms | 0.4 ms | 0.0 ms | 0.7 ms | 0.8 ms | 35.3 ms | 4.6 ms | 4.0 ms | 50.9 ms | 7.1 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 0.3 ms | 0.0 ms | 0.0 ms | 0.01× | 0.10× | 0.5 ms | 0.0 ms | 0.2 ms | 45.5 ms | 3.7 ms | 4.1 ms | 0.08× | 0.09× |
| tour | 3.5 ms | 0.1 ms | 0.7 ms | 0.02× | 0.21× | 0.7 ms | 0.0 ms | 1.1 ms | 56.2 ms | 3.9 ms | 5.7 ms | 0.08× | 0.11× |
| functions | 2.9 ms | 0.0 ms | 0.6 ms | 0.01× | 0.21× | 0.9 ms | 0.0 ms | 3.1 ms | 48.3 ms | 3.7 ms | 7.5 ms | 0.08× | 0.14× |
| errors | 1.3 ms | 0.0 ms | 0.6 ms | 0.02× | 0.42× | 0.6 ms | 0.0 ms | 0.9 ms | 42.7 ms | 3.5 ms | 4.8 ms | 0.08× | 0.11× |
| sieve | 7.4 ms | 0.6 ms | 61.5 ms | 0.08× | 8.39× | 0.6 ms | 0.0 ms | 0.4 ms | 52.4 ms | 4.3 ms | 65.6 ms | 0.08× | 1.25× |
| sorting | 637.5 ms | 132.7 ms | 24.8 s | 0.21× | 38× | 1.0 ms | 0.0 ms | 3.5 ms | 686.2 ms | 137.4 ms | 24.9 s | 0.20× | 35× |
| life | 26.8 ms | 5.2 ms | 1,874.1 ms | 0.19× | 69× | 0.6 ms | 0.0 ms | 0.6 ms | 75.2 ms | 9.3 ms | 1,879.5 ms | 0.12× | 25× |
| maze | 1.9 ms | 0.0 ms | 2.6 ms | 0.02× | 1.41× | 0.7 ms | 0.0 ms | 1.1 ms | 47.1 ms | 3.7 ms | 7.3 ms | 0.08× | 0.15× |
| sudoku | 15.2 ms | 8.0 ms | 1,050.2 ms | 0.51× | 69× | 0.6 ms | 0.0 ms | 0.5 ms | 62.4 ms | 11.6 ms | 1,055.0 ms | 0.19× | 17× |
| vm | 1.9 ms | 0.0 ms | 1.9 ms | 0.02× | 1.06× | 0.7 ms | 0.0 ms | 0.8 ms | 50.9 ms | 4.0 ms | 7.1 ms | 0.08× | 0.14× |

### results/aot-0c142ba/compiler.json

5 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| compile-squares | 0.9 ms | 1.1 ms | 0.9 ms | 71.4 ms | 27.1 ms | 1.14× | 0.99× | 74× | 30× | match |
| compile-vm | 5.1 ms | 5.2 ms | 4.7 ms | 207.1 ms | 497.0 ms | 1.04× | 0.95× | 41× | 104× | match |
| compile-sudoku | 4.0 ms | 4.2 ms | 4.0 ms | 199.3 ms | 369.9 ms | 1.08× | 0.96× | 49× | 93× | match |
| compile-functions | 19.7 ms | 22.2 ms | 20.8 ms | 334.3 ms | 2,224.2 ms | 1.12× | 1.13× | 18× | 117× | match |
| compile64-vm | 8.3 ms | 8.4 ms | 10.3 ms | 916.6 ms | 556.7 ms | 1.05× | 1.28× | 116× | 67× | match |
| compile64-functions | 32.0 ms | 33.2 ms | 38.2 ms | 1,200.3 ms | 2,477.2 ms | 1.02× | 1.19× | 36× | 76× | match |
| compile64-8g-vm | 8.2 ms | 8.5 ms | 8.0 ms | 329.1 ms | 541.0 ms | 1.07× | 0.96× | 40× | 66× | match |
| compile64-8g-functions | 32.8 ms | 36.1 ms | 31.2 ms | 486.4 ms | 2,428.2 ms | 1.10× | 0.95× | 15× | 73× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 3,428.0 ms | 4.0 ms | 0.3 ms | 30.5 ms | 110.5 ms | 3,441.3 ms | 12.3 ms | 5.0 ms | 196.1 ms | 147.5 ms |
| compile-vm | 3,426.6 ms | 4.1 ms | 0.3 ms | 28.3 ms | 118.6 ms | 3,442.6 ms | 16.7 ms | 9.4 ms | 354.6 ms | 632.4 ms |
| compile-sudoku | 3,471.6 ms | 4.1 ms | 0.3 ms | 29.0 ms | 117.5 ms | 3,486.5 ms | 15.7 ms | 8.6 ms | 343.5 ms | 499.4 ms |
| compile-functions | 3,530.7 ms | 4.2 ms | 0.3 ms | 38.9 ms | 115.5 ms | 3,561.9 ms | 33.8 ms | 25.3 ms | 483.8 ms | 2,348.3 ms |
| compile64-vm | 5,502.8 ms | 4.4 ms | 0.3 ms | 31.1 ms | 139.7 ms | 5,524.6 ms | 20.1 ms | 15.2 ms | 1,049.9 ms | 706.0 ms |
| compile64-functions | 5,322.5 ms | 4.4 ms | 0.3 ms | 30.5 ms | 135.0 ms | 5,368.4 ms | 45.9 ms | 43.4 ms | 1,341.7 ms | 2,626.7 ms |
| compile64-8g-vm | 5,294.5 ms | 4.6 ms | 0.3 ms | 34.3 ms | 137.3 ms | 5,315.5 ms | 21.7 ms | 12.5 ms | 482.9 ms | 690.9 ms |
| compile64-8g-functions | 5,364.0 ms | 4.6 ms | 0.3 ms | 34.4 ms | 133.7 ms | 5,413.3 ms | 50.0 ms | 36.5 ms | 631.3 ms | 2,582.8 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 71.4 ms | 0.9 ms | 27.1 ms | 0.01× | 0.37× | 30.5 ms | 0.3 ms | 110.5 ms | 196.1 ms | 5.0 ms | 147.5 ms | 0.03× | 0.78× |
| compile-vm | 207.1 ms | 4.7 ms | 497.0 ms | 0.02× | 2.42× | 28.3 ms | 0.3 ms | 118.6 ms | 354.6 ms | 9.4 ms | 632.4 ms | 0.03× | 1.83× |
| compile-sudoku | 199.3 ms | 4.0 ms | 369.9 ms | 0.02× | 1.89× | 29.0 ms | 0.3 ms | 117.5 ms | 343.5 ms | 8.6 ms | 499.4 ms | 0.03× | 1.47× |
| compile-functions | 334.3 ms | 20.8 ms | 2,224.2 ms | 0.06× | 6.57× | 38.9 ms | 0.3 ms | 115.5 ms | 483.8 ms | 25.3 ms | 2,348.3 ms | 0.05× | 4.86× |
| compile64-vm | 916.6 ms | 10.3 ms | 556.7 ms | 0.01× | 0.61× | 31.1 ms | 0.3 ms | 139.7 ms | 1,049.9 ms | 15.2 ms | 706.0 ms | 0.01× | 0.67× |
| compile64-functions | 1,200.3 ms | 38.2 ms | 2,477.2 ms | 0.03× | 2.08× | 30.5 ms | 0.3 ms | 135.0 ms | 1,341.7 ms | 43.4 ms | 2,626.7 ms | 0.03× | 1.96× |
| compile64-8g-vm | 329.1 ms | 8.0 ms | 541.0 ms | 0.02× | 1.66× | 34.3 ms | 0.3 ms | 137.3 ms | 482.9 ms | 12.5 ms | 690.9 ms | 0.03× | 1.45× |
| compile64-8g-functions | 486.4 ms | 31.2 ms | 2,428.2 ms | 0.06× | 5.01× | 34.4 ms | 0.3 ms | 133.7 ms | 631.3 ms | 36.5 ms | 2,582.8 ms | 0.06× | 4.13× |

