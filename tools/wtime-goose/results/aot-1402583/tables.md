### results/aot-1402583/examples.json

5 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.0 ms | 0.0 ms | 0.3 ms | 0.0 ms | 1.11× | 0.11× | 14× | 1.46× | match |
| tour | 0.1 ms | 0.1 ms | 0.0 ms | 3.2 ms | 0.8 ms | 1.33× | 0.66× | 39× | 9.34× | match |
| functions | 0.1 ms | 0.1 ms | 0.0 ms | 2.8 ms | 0.6 ms | 1.30× | 0.61× | 44× | 8.98× | match |
| errors | 0.0 ms | 0.1 ms | 0.0 ms | 1.4 ms | 0.6 ms | 1.09× | 0.47× | 29× | 13× | match |
| sieve | 0.7 ms | 0.7 ms | 0.8 ms | 6.7 ms | 64.7 ms | 0.93× | 1.17× | 9.40× | 100× | match |
| sorting | 191.5 ms | 176.6 ms | 128.6 ms | 233.1 ms | 23.8 s | 0.99× | 0.68× | 1.22× | 124× | match |
| life | 22.5 ms | 22.4 ms | 13.7 ms | 25.9 ms | 1,843.9 ms | 0.99× | 0.59× | 1.16× | 78× | match |
| maze | 0.1 ms | 0.1 ms | 0.0 ms | 1.7 ms | 2.5 ms | 1.18× | 0.68× | 25× | 35× | match |
| sudoku | 9.2 ms | 10.2 ms | 9.6 ms | 15.5 ms | 1,144.0 ms | 1.02× | 0.99× | 1.64× | 124× | match |
| vm | 0.1 ms | 0.1 ms | 0.1 ms | 2.8 ms | 2.6 ms | 1.04× | 0.70× | 30× | 31× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 4.8 ms | 0.3 ms | 0.0 ms | 0.5 ms | 0.2 ms | 9.2 ms | 5.3 ms | 4.2 ms | 46.4 ms | 4.5 ms |
| tour | 41.2 ms | 0.4 ms | 0.0 ms | 0.7 ms | 1.1 ms | 45.6 ms | 5.0 ms | 4.2 ms | 52.9 ms | 6.0 ms |
| functions | 127.1 ms | 0.5 ms | 0.0 ms | 0.9 ms | 2.9 ms | 131.8 ms | 5.3 ms | 4.1 ms | 48.9 ms | 7.1 ms |
| errors | 32.6 ms | 0.3 ms | 0.0 ms | 0.6 ms | 0.9 ms | 37.7 ms | 4.7 ms | 3.7 ms | 47.1 ms | 5.1 ms |
| sieve | 11.9 ms | 0.3 ms | 0.0 ms | 0.6 ms | 0.4 ms | 16.7 ms | 5.3 ms | 4.6 ms | 54.0 ms | 69.4 ms |
| sorting | 103.2 ms | 0.5 ms | 0.0 ms | 0.9 ms | 3.1 ms | 308.5 ms | 182.0 ms | 133.6 ms | 282.1 ms | 23.8 s |
| life | 18.5 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.5 ms | 45.8 ms | 27.3 ms | 18.4 ms | 75.1 ms | 1,849.5 ms |
| maze | 43.5 ms | 0.4 ms | 0.0 ms | 0.7 ms | 1.1 ms | 49.5 ms | 4.9 ms | 4.0 ms | 47.8 ms | 7.3 ms |
| sudoku | 16.7 ms | 0.4 ms | 0.0 ms | 0.6 ms | 0.5 ms | 30.6 ms | 15.5 ms | 13.8 ms | 65.1 ms | 1,149.1 ms |
| vm | 41.3 ms | 0.5 ms | 0.0 ms | 0.9 ms | 1.0 ms | 46.8 ms | 5.7 ms | 4.4 ms | 70.3 ms | 8.6 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 0.3 ms | 0.0 ms | 0.0 ms | 0.01× | 0.11× | 0.5 ms | 0.0 ms | 0.2 ms | 46.4 ms | 4.2 ms | 4.5 ms | 0.09× | 0.10× |
| tour | 3.2 ms | 0.0 ms | 0.8 ms | 0.01× | 0.24× | 0.7 ms | 0.0 ms | 1.1 ms | 52.9 ms | 4.2 ms | 6.0 ms | 0.08× | 0.12× |
| functions | 2.8 ms | 0.0 ms | 0.6 ms | 0.01× | 0.21× | 0.9 ms | 0.0 ms | 2.9 ms | 48.9 ms | 4.1 ms | 7.1 ms | 0.08× | 0.15× |
| errors | 1.4 ms | 0.0 ms | 0.6 ms | 0.02× | 0.45× | 0.6 ms | 0.0 ms | 0.9 ms | 47.1 ms | 3.7 ms | 5.1 ms | 0.08× | 0.11× |
| sieve | 6.7 ms | 0.8 ms | 64.7 ms | 0.12× | 9.59× | 0.6 ms | 0.0 ms | 0.4 ms | 54.0 ms | 4.6 ms | 69.4 ms | 0.09× | 1.27× |
| sorting | 233.1 ms | 128.6 ms | 23.8 s | 0.54× | 102× | 0.9 ms | 0.0 ms | 3.1 ms | 282.1 ms | 133.6 ms | 23.8 s | 0.46× | 84× |
| life | 25.9 ms | 13.7 ms | 1,843.9 ms | 0.52× | 69× | 0.6 ms | 0.0 ms | 0.5 ms | 75.1 ms | 18.4 ms | 1,849.5 ms | 0.24× | 24× |
| maze | 1.7 ms | 0.0 ms | 2.5 ms | 0.03× | 1.44× | 0.7 ms | 0.0 ms | 1.1 ms | 47.8 ms | 4.0 ms | 7.3 ms | 0.08× | 0.16× |
| sudoku | 15.5 ms | 9.6 ms | 1,144.0 ms | 0.63× | 76× | 0.6 ms | 0.0 ms | 0.5 ms | 65.1 ms | 13.8 ms | 1,149.1 ms | 0.23× | 18× |
| vm | 2.8 ms | 0.1 ms | 2.6 ms | 0.02× | 1.00× | 0.9 ms | 0.0 ms | 1.0 ms | 70.3 ms | 4.4 ms | 8.6 ms | 0.08× | 0.13× |

### results/aot-1402583/compiler.json

5 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| compile-squares | 1.1 ms | 1.3 ms | 1.5 ms | 111.8 ms | 27.9 ms | 1.25× | 1.22× | 109× | 27× | match |
| compile-vm | 5.8 ms | 6.2 ms | 6.9 ms | 376.4 ms | 423.2 ms | 1.07× | 1.12× | 63× | 72× | match |
| compile-sudoku | 4.6 ms | 4.9 ms | 5.1 ms | 350.7 ms | 347.8 ms | 1.11× | 1.07× | 78× | 74× | match |
| compile-functions | 22.8 ms | 23.1 ms | 25.9 ms | 533.4 ms | 1,883.0 ms | 0.99× | 1.14× | 22× | 84× | match |
| compile64-vm | 8.7 ms | 11.6 ms | 17.1 ms | 1,132.8 ms | 596.5 ms | 1.06× | 1.81× | 130× | 69× | match |
| compile64-functions | 35.2 ms | 34.0 ms | 58.4 ms | 1,577.7 ms | 2,776.4 ms | 1.00× | 1.63× | 46× | 81× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 5,970.8 ms | 6.4 ms | 0.3 ms | 54.9 ms | 200.4 ms | 5,986.4 ms | 17.8 ms | 6.6 ms | 283.4 ms | 244.8 ms |
| compile-vm | 6,165.5 ms | 6.2 ms | 0.3 ms | 46.8 ms | 193.2 ms | 6,187.7 ms | 21.3 ms | 11.8 ms | 539.8 ms | 633.2 ms |
| compile-sudoku | 5,987.5 ms | 5.8 ms | 0.2 ms | 45.9 ms | 196.1 ms | 6,007.0 ms | 19.8 ms | 10.0 ms | 512.2 ms | 562.2 ms |
| compile-functions | 5,794.9 ms | 6.1 ms | 0.3 ms | 39.6 ms | 196.1 ms | 5,834.1 ms | 38.5 ms | 32.6 ms | 680.2 ms | 2,142.1 ms |
| compile64-vm | 9,754.5 ms | 7.1 ms | 0.3 ms | 53.0 ms | 235.7 ms | 9,787.1 ms | 30.1 ms | 22.3 ms | 1,316.2 ms | 853.3 ms |
| compile64-functions | 9,970.9 ms | 6.7 ms | 0.3 ms | 52.8 ms | 238.7 ms | 10.0 s | 52.3 ms | 64.4 ms | 1,736.6 ms | 3,085.3 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 111.8 ms | 1.5 ms | 27.9 ms | 0.01× | 0.25× | 54.9 ms | 0.3 ms | 200.4 ms | 283.4 ms | 6.6 ms | 244.8 ms | 0.02× | 0.89× |
| compile-vm | 376.4 ms | 6.9 ms | 423.2 ms | 0.02× | 1.13× | 46.8 ms | 0.3 ms | 193.2 ms | 539.8 ms | 11.8 ms | 633.2 ms | 0.02× | 1.20× |
| compile-sudoku | 350.7 ms | 5.1 ms | 347.8 ms | 0.01× | 0.95× | 45.9 ms | 0.2 ms | 196.1 ms | 512.2 ms | 10.0 ms | 562.2 ms | 0.02× | 1.10× |
| compile-functions | 533.4 ms | 25.9 ms | 1,883.0 ms | 0.05× | 3.60× | 39.6 ms | 0.3 ms | 196.1 ms | 680.2 ms | 32.6 ms | 2,142.1 ms | 0.05× | 3.08× |
| compile64-vm | 1,132.8 ms | 17.1 ms | 596.5 ms | 0.02× | 0.53× | 53.0 ms | 0.3 ms | 235.7 ms | 1,316.2 ms | 22.3 ms | 853.3 ms | 0.02× | 0.65× |
| compile64-functions | 1,577.7 ms | 58.4 ms | 2,776.4 ms | 0.04× | 1.76× | 52.8 ms | 0.3 ms | 238.7 ms | 1,736.6 ms | 64.4 ms | 3,085.3 ms | 0.04× | 1.75× |

