### results/aot-5be3c27/examples.json

5 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.0 ms | 0.0 ms | 0.4 ms | 0.0 ms | 1.07× | 0.11× | 20× | 2.31× | match |
| tour | 0.1 ms | 0.1 ms | 0.1 ms | 3.5 ms | 0.7 ms | 0.99× | 0.65× | 46× | 8.84× | match |
| functions | 0.1 ms | 0.1 ms | 0.0 ms | 3.3 ms | 0.6 ms | 0.81× | 0.49× | 40× | 8.76× | match |
| errors | 0.1 ms | 0.1 ms | 0.0 ms | 1.6 ms | 0.6 ms | 1.03× | 0.52× | 30× | 12× | match |
| sieve | 0.9 ms | 0.8 ms | 0.9 ms | 7.1 ms | 81.6 ms | 0.96× | 1.04× | 8.22× | 96× | match |
| sorting | 182.7 ms | 197.1 ms | 134.1 ms | 656.8 ms | 27.4 s | 1.02× | 0.68× | 3.59× | 146× | match |
| life | 22.9 ms | 23.1 ms | 14.1 ms | 26.4 ms | 1,967.0 ms | 0.97× | 0.60× | 1.15× | 83× | match |
| maze | 0.1 ms | 0.1 ms | 0.1 ms | 2.2 ms | 4.2 ms | 1.10× | 0.74× | 25× | 42× | match |
| sudoku | 14.2 ms | 13.3 ms | 12.2 ms | 23.0 ms | 1,442.3 ms | 0.98× | 0.84× | 1.54× | 93× | match |
| vm | 0.1 ms | 0.1 ms | 0.1 ms | 2.6 ms | 2.5 ms | 0.97× | 0.60× | 27× | 27× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 4.3 ms | 0.3 ms | 0.0 ms | 0.6 ms | 0.2 ms | 8.3 ms | 4.3 ms | 3.8 ms | 53.1 ms | 5.2 ms |
| tour | 43.5 ms | 0.4 ms | 0.0 ms | 0.8 ms | 1.1 ms | 48.0 ms | 4.8 ms | 4.3 ms | 52.4 ms | 5.7 ms |
| functions | 139.7 ms | 0.5 ms | 0.0 ms | 1.0 ms | 3.1 ms | 144.6 ms | 5.3 ms | 4.2 ms | 53.6 ms | 7.7 ms |
| errors | 37.7 ms | 0.4 ms | 0.0 ms | 0.7 ms | 1.0 ms | 42.6 ms | 5.6 ms | 4.5 ms | 57.8 ms | 6.0 ms |
| sieve | 13.7 ms | 0.4 ms | 0.0 ms | 0.7 ms | 0.5 ms | 18.7 ms | 5.6 ms | 5.1 ms | 67.7 ms | 86.9 ms |
| sorting | 119.4 ms | 0.5 ms | 0.0 ms | 1.0 ms | 3.4 ms | 312.6 ms | 203.0 ms | 139.5 ms | 705.0 ms | 27.4 s |
| life | 17.9 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.5 ms | 46.2 ms | 28.2 ms | 18.3 ms | 75.4 ms | 1,972.3 ms |
| maze | 47.3 ms | 0.5 ms | 0.0 ms | 0.7 ms | 1.1 ms | 52.8 ms | 6.1 ms | 4.8 ms | 53.1 ms | 10.9 ms |
| sudoku | 22.5 ms | 0.5 ms | 0.0 ms | 0.7 ms | 0.7 ms | 42.6 ms | 18.8 ms | 17.4 ms | 83.2 ms | 1,448.1 ms |
| vm | 44.1 ms | 0.5 ms | 0.0 ms | 0.9 ms | 1.1 ms | 49.8 ms | 5.9 ms | 4.9 ms | 67.6 ms | 8.6 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 0.4 ms | 0.0 ms | 0.0 ms | 0.01× | 0.10× | 0.6 ms | 0.0 ms | 0.2 ms | 53.1 ms | 3.8 ms | 5.2 ms | 0.07× | 0.10× |
| tour | 3.5 ms | 0.1 ms | 0.7 ms | 0.01× | 0.19× | 0.8 ms | 0.0 ms | 1.1 ms | 52.4 ms | 4.3 ms | 5.7 ms | 0.08× | 0.11× |
| functions | 3.3 ms | 0.0 ms | 0.6 ms | 0.01× | 0.19× | 1.0 ms | 0.0 ms | 3.1 ms | 53.6 ms | 4.2 ms | 7.7 ms | 0.08× | 0.14× |
| errors | 1.6 ms | 0.0 ms | 0.6 ms | 0.02× | 0.37× | 0.7 ms | 0.0 ms | 1.0 ms | 57.8 ms | 4.5 ms | 6.0 ms | 0.08× | 0.12× |
| sieve | 7.1 ms | 0.9 ms | 81.6 ms | 0.12× | 15× | 0.7 ms | 0.0 ms | 0.5 ms | 67.7 ms | 5.1 ms | 86.9 ms | 0.08× | 1.29× |
| sorting | 656.8 ms | 134.1 ms | 27.4 s | 0.19× | 41× | 1.0 ms | 0.0 ms | 3.4 ms | 705.0 ms | 139.5 ms | 27.4 s | 0.18× | 38× |
| life | 26.4 ms | 14.1 ms | 1,967.0 ms | 0.53× | 70× | 0.6 ms | 0.0 ms | 0.5 ms | 75.4 ms | 18.3 ms | 1,972.3 ms | 0.24× | 24× |
| maze | 2.2 ms | 0.1 ms | 4.2 ms | 0.03× | 1.63× | 0.7 ms | 0.0 ms | 1.1 ms | 53.1 ms | 4.8 ms | 10.9 ms | 0.09× | 0.17× |
| sudoku | 23.0 ms | 12.2 ms | 1,442.3 ms | 0.57× | 65× | 0.7 ms | 0.0 ms | 0.7 ms | 83.2 ms | 17.4 ms | 1,448.1 ms | 0.21× | 17× |
| vm | 2.6 ms | 0.1 ms | 2.5 ms | 0.02× | 0.99× | 0.9 ms | 0.0 ms | 1.1 ms | 67.6 ms | 4.9 ms | 8.6 ms | 0.07× | 0.13× |

### results/aot-5be3c27/compiler.json

5 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| compile-squares | 1.1 ms | 1.4 ms | 1.7 ms | 152.3 ms | 39.8 ms | 1.07× | 1.41× | 125× | 32× | match |
| compile-vm | 6.2 ms | 6.1 ms | 7.3 ms | 505.9 ms | 638.8 ms | 0.99× | 1.19× | 80× | 103× | match |
| compile-sudoku | 4.9 ms | 5.8 ms | 5.6 ms | 395.0 ms | 429.3 ms | 1.05× | 1.14× | 81× | 69× | match |
| compile-functions | 34.1 ms | 34.0 ms | 40.5 ms | 742.0 ms | 2,498.8 ms | 1.06× | 1.18× | 23× | 73× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 7,684.9 ms | 6.9 ms | 0.3 ms | 75.2 ms | 251.8 ms | 7,703.5 ms | 18.5 ms | 6.8 ms | 343.9 ms | 314.2 ms |
| compile-vm | 6,877.3 ms | 6.8 ms | 0.3 ms | 65.5 ms | 241.7 ms | 6,902.7 ms | 24.2 ms | 12.4 ms | 685.7 ms | 902.5 ms |
| compile-sudoku | 6,561.5 ms | 6.4 ms | 0.3 ms | 47.8 ms | 241.7 ms | 6,586.7 ms | 23.5 ms | 10.9 ms | 566.6 ms | 689.8 ms |
| compile-functions | 6,796.1 ms | 6.5 ms | 0.3 ms | 66.0 ms | 220.3 ms | 6,840.6 ms | 52.4 ms | 47.1 ms | 927.4 ms | 2,739.5 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 152.3 ms | 1.7 ms | 39.8 ms | 0.01× | 0.25× | 75.2 ms | 0.3 ms | 251.8 ms | 343.9 ms | 6.8 ms | 314.2 ms | 0.02× | 0.87× |
| compile-vm | 505.9 ms | 7.3 ms | 638.8 ms | 0.02× | 1.28× | 65.5 ms | 0.3 ms | 241.7 ms | 685.7 ms | 12.4 ms | 902.5 ms | 0.02× | 1.32× |
| compile-sudoku | 395.0 ms | 5.6 ms | 429.3 ms | 0.02× | 0.89× | 47.8 ms | 0.3 ms | 241.7 ms | 566.6 ms | 10.9 ms | 689.8 ms | 0.02× | 1.00× |
| compile-functions | 742.0 ms | 40.5 ms | 2,498.8 ms | 0.05× | 3.41× | 66.0 ms | 0.3 ms | 220.3 ms | 927.4 ms | 47.1 ms | 2,739.5 ms | 0.05× | 2.98× |

