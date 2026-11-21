### results/aot-7042670/examples.json

5 rounds, CPU 1 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.0 ms | 0.0 ms | 0.4 ms | 0.0 ms | 0.92× | 0.11× | 14× | 1.63× | match |
| tour | 0.1 ms | 0.1 ms | 0.1 ms | 3.7 ms | 0.8 ms | 0.97× | 0.70× | 48× | 10× | match |
| functions | 0.1 ms | 0.1 ms | 0.0 ms | 3.3 ms | 0.6 ms | 1.15× | 0.64× | 43× | 7.74× | match |
| errors | 0.0 ms | 0.1 ms | 0.0 ms | 1.6 ms | 0.7 ms | 1.26× | 0.52× | 34× | 13× | match |
| sieve | 0.7 ms | 0.8 ms | 0.6 ms | 7.4 ms | 70.3 ms | 1.07× | 0.87× | 9.95× | 101× | match |
| sorting | 223.5 ms | 196.1 ms | 131.8 ms | 275.8 ms | 27.4 s | 0.88× | 0.59× | 1.24× | 122× | match |
| life | 24.9 ms | 24.3 ms | 5.2 ms | 28.0 ms | 2,043.2 ms | 0.97× | 0.21× | 1.13× | 82× | match |
| maze | 0.1 ms | 0.1 ms | 0.0 ms | 1.9 ms | 2.9 ms | 1.06× | 0.58× | 26× | 39× | match |
| sudoku | 10.3 ms | 10.0 ms | 8.2 ms | 17.5 ms | 1,295.4 ms | 0.99× | 0.82× | 1.63× | 122× | match |
| vm | 0.1 ms | 0.1 ms | 0.0 ms | 1.9 ms | 1.9 ms | 0.99× | 0.57× | 30× | 26× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 4.8 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.2 ms | 9.6 ms | 5.6 ms | 4.2 ms | 63.9 ms | 5.1 ms |
| tour | 41.9 ms | 0.5 ms | 0.0 ms | 0.7 ms | 1.1 ms | 46.7 ms | 4.8 ms | 4.7 ms | 62.1 ms | 6.4 ms |
| functions | 149.9 ms | 0.5 ms | 0.0 ms | 1.0 ms | 3.3 ms | 155.5 ms | 5.4 ms | 4.1 ms | 55.4 ms | 8.1 ms |
| errors | 37.2 ms | 0.4 ms | 0.0 ms | 0.8 ms | 1.1 ms | 42.3 ms | 5.0 ms | 4.7 ms | 52.8 ms | 5.8 ms |
| sieve | 13.1 ms | 0.3 ms | 0.0 ms | 0.6 ms | 0.4 ms | 18.8 ms | 5.5 ms | 4.7 ms | 61.1 ms | 76.1 ms |
| sorting | 118.8 ms | 0.5 ms | 0.0 ms | 1.0 ms | 3.3 ms | 350.6 ms | 201.5 ms | 137.2 ms | 328.2 ms | 27.4 s |
| life | 19.0 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.6 ms | 49.4 ms | 29.4 ms | 9.6 ms | 78.7 ms | 2,048.4 ms |
| maze | 40.7 ms | 0.4 ms | 0.0 ms | 0.8 ms | 1.2 ms | 45.4 ms | 4.6 ms | 4.2 ms | 50.8 ms | 9.4 ms |
| sudoku | 17.8 ms | 0.4 ms | 0.0 ms | 0.7 ms | 0.7 ms | 33.5 ms | 15.2 ms | 12.5 ms | 70.1 ms | 1,302.8 ms |
| vm | 32.3 ms | 0.4 ms | 0.0 ms | 0.7 ms | 0.8 ms | 38.6 ms | 4.7 ms | 4.0 ms | 53.7 ms | 7.1 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 0.4 ms | 0.0 ms | 0.0 ms | 0.01× | 0.10× | 0.6 ms | 0.0 ms | 0.2 ms | 63.9 ms | 4.2 ms | 5.1 ms | 0.07× | 0.08× |
| tour | 3.7 ms | 0.1 ms | 0.8 ms | 0.02× | 0.23× | 0.7 ms | 0.0 ms | 1.1 ms | 62.1 ms | 4.7 ms | 6.4 ms | 0.07× | 0.11× |
| functions | 3.3 ms | 0.0 ms | 0.6 ms | 0.01× | 0.20× | 1.0 ms | 0.0 ms | 3.3 ms | 55.4 ms | 4.1 ms | 8.1 ms | 0.07× | 0.15× |
| errors | 1.6 ms | 0.0 ms | 0.7 ms | 0.02× | 0.40× | 0.8 ms | 0.0 ms | 1.1 ms | 52.8 ms | 4.7 ms | 5.8 ms | 0.08× | 0.11× |
| sieve | 7.4 ms | 0.6 ms | 70.3 ms | 0.09× | 10× | 0.6 ms | 0.0 ms | 0.4 ms | 61.1 ms | 4.7 ms | 76.1 ms | 0.08× | 1.31× |
| sorting | 275.8 ms | 131.8 ms | 27.4 s | 0.48× | 101× | 1.0 ms | 0.0 ms | 3.3 ms | 328.2 ms | 137.2 ms | 27.4 s | 0.42× | 85× |
| life | 28.0 ms | 5.2 ms | 2,043.2 ms | 0.19× | 73× | 0.6 ms | 0.0 ms | 0.6 ms | 78.7 ms | 9.6 ms | 2,048.4 ms | 0.12× | 26× |
| maze | 1.9 ms | 0.0 ms | 2.9 ms | 0.02× | 1.50× | 0.8 ms | 0.0 ms | 1.2 ms | 50.8 ms | 4.2 ms | 9.4 ms | 0.08× | 0.17× |
| sudoku | 17.5 ms | 8.2 ms | 1,295.4 ms | 0.49× | 74× | 0.7 ms | 0.0 ms | 0.7 ms | 70.1 ms | 12.5 ms | 1,302.8 ms | 0.19× | 19× |
| vm | 1.9 ms | 0.0 ms | 1.9 ms | 0.02× | 0.95× | 0.7 ms | 0.0 ms | 0.8 ms | 53.7 ms | 4.0 ms | 7.1 ms | 0.08× | 0.13× |

### results/aot-7042670/compiler.json

5 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| compile-squares | 0.9 ms | 0.9 ms | 1.0 ms | 73.4 ms | 28.1 ms | 0.98× | 1.05× | 81× | 30× | match |
| compile-vm | 5.2 ms | 5.0 ms | 5.0 ms | 215.0 ms | 508.4 ms | 0.97× | 0.96× | 41× | 97× | match |
| compile-sudoku | 4.7 ms | 4.6 ms | 4.5 ms | 223.1 ms | 442.8 ms | 0.94× | 0.96× | 46× | 92× | match |
| compile-functions | 21.5 ms | 21.3 ms | 20.1 ms | 388.5 ms | 2,326.9 ms | 1.00× | 0.98× | 18× | 111× | match |
| compile64-vm | 8.6 ms | 8.7 ms | 10.8 ms | 1,027.5 ms | 635.3 ms | 0.99× | 1.24× | 112× | 69× | match |
| compile64-functions | 37.1 ms | 35.8 ms | 41.1 ms | 1,271.3 ms | 2,692.6 ms | 0.98× | 1.09× | 36× | 77× | match |
| compile64-8g-vm | 8.5 ms | 8.5 ms | 8.4 ms | 350.6 ms | 565.6 ms | 0.98× | 0.98× | 41× | 67× | match |
| compile64-8g-functions | 34.6 ms | 34.7 ms | 32.4 ms | 501.5 ms | 2,638.4 ms | 0.99× | 0.94× | 14× | 77× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 3,600.9 ms | 4.0 ms | 0.3 ms | 27.7 ms | 118.2 ms | 3,613.1 ms | 12.6 ms | 5.5 ms | 198.3 ms | 157.3 ms |
| compile-vm | 3,673.1 ms | 4.1 ms | 0.3 ms | 29.6 ms | 135.5 ms | 3,690.1 ms | 16.7 ms | 10.0 ms | 369.6 ms | 644.3 ms |
| compile-sudoku | 3,652.0 ms | 4.3 ms | 0.3 ms | 36.0 ms | 132.3 ms | 3,670.5 ms | 19.7 ms | 9.5 ms | 397.0 ms | 597.3 ms |
| compile-functions | 3,671.5 ms | 4.1 ms | 0.3 ms | 32.4 ms | 123.0 ms | 3,705.9 ms | 33.6 ms | 25.7 ms | 556.1 ms | 2,463.2 ms |
| compile64-vm | 6,078.6 ms | 4.7 ms | 0.3 ms | 35.0 ms | 152.1 ms | 6,104.0 ms | 21.5 ms | 16.4 ms | 1,199.3 ms | 789.2 ms |
| compile64-functions | 6,084.8 ms | 4.8 ms | 0.3 ms | 34.7 ms | 141.1 ms | 6,135.4 ms | 48.8 ms | 46.7 ms | 1,417.3 ms | 2,847.3 ms |
| compile64-8g-vm | 5,704.7 ms | 4.5 ms | 0.3 ms | 34.4 ms | 169.0 ms | 5,728.3 ms | 21.2 ms | 14.0 ms | 508.8 ms | 748.8 ms |
| compile64-8g-functions | 5,728.4 ms | 4.6 ms | 0.3 ms | 35.1 ms | 141.3 ms | 5,785.2 ms | 47.7 ms | 38.5 ms | 650.7 ms | 2,816.1 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 73.4 ms | 1.0 ms | 28.1 ms | 0.01× | 0.38× | 27.7 ms | 0.3 ms | 118.2 ms | 198.3 ms | 5.5 ms | 157.3 ms | 0.03× | 0.78× |
| compile-vm | 215.0 ms | 5.0 ms | 508.4 ms | 0.02× | 2.32× | 29.6 ms | 0.3 ms | 135.5 ms | 369.6 ms | 10.0 ms | 644.3 ms | 0.03× | 1.79× |
| compile-sudoku | 223.1 ms | 4.5 ms | 442.8 ms | 0.02× | 1.98× | 36.0 ms | 0.3 ms | 132.3 ms | 397.0 ms | 9.5 ms | 597.3 ms | 0.03× | 1.53× |
| compile-functions | 388.5 ms | 20.1 ms | 2,326.9 ms | 0.06× | 5.96× | 32.4 ms | 0.3 ms | 123.0 ms | 556.1 ms | 25.7 ms | 2,463.2 ms | 0.05× | 4.61× |
| compile64-vm | 1,027.5 ms | 10.8 ms | 635.3 ms | 0.01× | 0.61× | 35.0 ms | 0.3 ms | 152.1 ms | 1,199.3 ms | 16.4 ms | 789.2 ms | 0.01× | 0.68× |
| compile64-functions | 1,271.3 ms | 41.1 ms | 2,692.6 ms | 0.03× | 2.11× | 34.7 ms | 0.3 ms | 141.1 ms | 1,417.3 ms | 46.7 ms | 2,847.3 ms | 0.03× | 2.03× |
| compile64-8g-vm | 350.6 ms | 8.4 ms | 565.6 ms | 0.02× | 1.64× | 34.4 ms | 0.3 ms | 169.0 ms | 508.8 ms | 14.0 ms | 748.8 ms | 0.03× | 1.46× |
| compile64-8g-functions | 501.5 ms | 32.4 ms | 2,638.4 ms | 0.07× | 5.26× | 35.1 ms | 0.3 ms | 141.3 ms | 650.7 ms | 38.5 ms | 2,816.1 ms | 0.06× | 4.36× |

