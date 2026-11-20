### results/v8-b2c2b04/examples.json

3 rounds, CPU 1 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | v8 run | wtime-decoded run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.5 ms | 0.0 ms | 16× | 1.10× | match |
| tour | 0.1 ms | 4.2 ms | 0.9 ms | 39× | 8.19× | match |
| functions | 0.1 ms | 4.5 ms | 0.8 ms | 51× | 9.82× | match |
| errors | 0.1 ms | 1.7 ms | 0.7 ms | 34× | 14× | match |
| sieve | 0.6 ms | 7.6 ms | 65.4 ms | 11× | 105× | match |
| sorting | 205.6 ms | 360.4 ms | 24.7 s | 2.00× | 116× | match |
| life | 22.0 ms | 25.6 ms | 2,024.6 ms | 1.15× | 91× | match |
| maze | 0.1 ms | 1.9 ms | 2.5 ms | 26× | 36× | match |
| sudoku | 9.8 ms | 17.7 ms | 1,157.3 ms | 1.60× | 113× | match |
| vm | 0.1 ms | 2.6 ms | 2.8 ms | 34× | 35× | match |

| Case | Wasmtime prep | v8 prep | wtime-decoded prep | Wasmtime process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|
| squares | 5.9 ms | 0.8 ms | 0.2 ms | 11.3 ms | 68.6 ms | 4.5 ms |
| tour | 62.0 ms | 1.0 ms | 1.3 ms | 68.2 ms | 70.7 ms | 6.5 ms |
| functions | 162.6 ms | 1.6 ms | 4.3 ms | 168.2 ms | 74.0 ms | 10.4 ms |
| errors | 52.0 ms | 0.8 ms | 1.2 ms | 57.6 ms | 49.5 ms | 5.5 ms |
| sieve | 11.6 ms | 0.7 ms | 0.4 ms | 16.0 ms | 56.2 ms | 69.4 ms |
| sorting | 135.4 ms | 1.4 ms | 3.3 ms | 345.9 ms | 434.5 ms | 24.7 s |
| life | 17.3 ms | 0.7 ms | 0.5 ms | 43.2 ms | 71.6 ms | 2,029.3 ms |
| maze | 37.1 ms | 0.8 ms | 1.1 ms | 41.6 ms | 46.5 ms | 6.9 ms |
| sudoku | 19.1 ms | 0.7 ms | 0.5 ms | 33.7 ms | 68.0 ms | 1,164.1 ms |
| vm | 42.2 ms | 0.9 ms | 1.0 ms | 48.0 ms | 69.7 ms | 8.5 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-decoded run | wtime-decoded / V8 run | V8 prep | wtime-decoded prep | V8 process | wtime-decoded process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 0.5 ms | 0.0 ms | 0.06× | 0.8 ms | 0.2 ms | 68.6 ms | 4.5 ms | 0.06× |
| tour | 4.2 ms | 0.9 ms | 0.21× | 1.0 ms | 1.3 ms | 70.7 ms | 6.5 ms | 0.10× |
| functions | 4.5 ms | 0.8 ms | 0.18× | 1.6 ms | 4.3 ms | 74.0 ms | 10.4 ms | 0.14× |
| errors | 1.7 ms | 0.7 ms | 0.41× | 0.8 ms | 1.2 ms | 49.5 ms | 5.5 ms | 0.10× |
| sieve | 7.6 ms | 65.4 ms | 10× | 0.7 ms | 0.4 ms | 56.2 ms | 69.4 ms | 1.24× |
| sorting | 360.4 ms | 24.7 s | 69× | 1.4 ms | 3.3 ms | 434.5 ms | 24.7 s | 58× |
| life | 25.6 ms | 2,024.6 ms | 80× | 0.7 ms | 0.5 ms | 71.6 ms | 2,029.3 ms | 29× |
| maze | 1.9 ms | 2.5 ms | 1.37× | 0.8 ms | 1.1 ms | 46.5 ms | 6.9 ms | 0.15× |
| sudoku | 17.7 ms | 1,157.3 ms | 65× | 0.7 ms | 0.5 ms | 68.0 ms | 1,164.1 ms | 18× |
| vm | 2.6 ms | 2.8 ms | 1.03× | 0.9 ms | 1.0 ms | 69.7 ms | 8.5 ms | 0.12× |

### results/v8-b2c2b04/compiler.json

3 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| compile-squares | 1.0 ms | 1.0 ms | 111.5 ms | 26.5 ms | 1.01× | 111× | 27× | match |
| compile-vm | 5.8 ms | 5.4 ms | 337.6 ms | 413.4 ms | 0.92× | 57× | 73× | match |
| compile-sudoku | 4.8 ms | 4.6 ms | 383.3 ms | 306.6 ms | 0.95× | 78× | 64× | match |
| compile-functions | 26.7 ms | 23.6 ms | 549.0 ms | 1,989.0 ms | 0.98× | 22× | 71× | match |
| compile64-vm | 8.8 ms | 8.3 ms | 1,224.5 ms | 639.0 ms | 0.95× | 132× | 73× | match |
| compile64-functions | 36.0 ms | 37.8 ms | 1,608.0 ms | 3,062.2 ms | 1.05× | 45× | 81× | match |

| Case | Wasmtime prep | wasmtime-cached prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 5,836.2 ms | 5.8 ms | 39.9 ms | 193.6 ms | 5,851.5 ms | 15.3 ms | 250.9 ms | 237.8 ms |
| compile-vm | 5,835.1 ms | 5.5 ms | 45.7 ms | 190.2 ms | 5,853.1 ms | 20.3 ms | 504.5 ms | 622.3 ms |
| compile-sudoku | 6,247.1 ms | 6.1 ms | 56.5 ms | 202.1 ms | 6,272.0 ms | 19.9 ms | 553.3 ms | 531.6 ms |
| compile-functions | 6,012.9 ms | 6.3 ms | 70.4 ms | 215.5 ms | 6,050.2 ms | 39.3 ms | 706.4 ms | 2,223.2 ms |
| compile64-vm | 10.1 s | 7.1 ms | 53.5 ms | 251.3 ms | 10.1 s | 25.5 ms | 1,404.0 ms | 950.3 ms |
| compile64-functions | 10.5 s | 8.2 ms | 54.6 ms | 295.9 ms | 10.6 s | 55.5 ms | 1,788.3 ms | 3,378.9 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-decoded run | wtime-decoded / V8 run | V8 prep | wtime-decoded prep | V8 process | wtime-decoded process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 111.5 ms | 26.5 ms | 0.24× | 39.9 ms | 193.6 ms | 250.9 ms | 237.8 ms | 0.95× |
| compile-vm | 337.6 ms | 413.4 ms | 1.21× | 45.7 ms | 190.2 ms | 504.5 ms | 622.3 ms | 1.24× |
| compile-sudoku | 383.3 ms | 306.6 ms | 0.78× | 56.5 ms | 202.1 ms | 553.3 ms | 531.6 ms | 0.94× |
| compile-functions | 549.0 ms | 1,989.0 ms | 3.44× | 70.4 ms | 215.5 ms | 706.4 ms | 2,223.2 ms | 2.97× |
| compile64-vm | 1,224.5 ms | 639.0 ms | 0.50× | 53.5 ms | 251.3 ms | 1,404.0 ms | 950.3 ms | 0.68× |
| compile64-functions | 1,608.0 ms | 3,062.2 ms | 1.98× | 54.6 ms | 295.9 ms | 1,788.3 ms | 3,378.9 ms | 1.96× |

