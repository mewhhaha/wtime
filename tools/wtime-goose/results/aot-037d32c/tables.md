### results/aot-037d32c/examples.json

5 rounds, CPU 1 (AMD Ryzen 7 3800X 8-Core Processor, THP disabled). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.0 ms | 0.0 ms | 0.2 ms | 0.0 ms | 0.77× | 0.08× | 9.64× | 1.03× | match |
| tour | 0.1 ms | 0.1 ms | 0.0 ms | 2.8 ms | 0.7 ms | 1.01× | 0.56× | 38× | 9.10× | match |
| functions | 0.1 ms | 0.1 ms | 0.0 ms | 2.6 ms | 0.7 ms | 1.09× | 0.50× | 42× | 12× | match |
| errors | 0.0 ms | 0.0 ms | 0.0 ms | 1.3 ms | 0.7 ms | 0.95× | 0.34× | 28× | 15× | match |
| sieve | 0.7 ms | 0.7 ms | 0.5 ms | 4.7 ms | 84.9 ms | 1.03× | 0.70× | 6.40× | 119× | match |
| sorting | 172.5 ms | 174.7 ms | 100.9 ms | 278.0 ms | 34.8 s | 1.03× | 0.59× | 1.61× | 206× | match |
| life | 24.4 ms | 24.0 ms | 5.8 ms | 27.2 ms | 2,360.6 ms | 0.98× | 0.24× | 1.11× | 97× | match |
| maze | 0.1 ms | 0.1 ms | 0.0 ms | 1.5 ms | 3.0 ms | 0.93× | 0.48× | 22× | 47× | match |
| sudoku | 10.0 ms | 10.2 ms | 4.4 ms | 16.4 ms | 1,467.3 ms | 1.04× | 0.45× | 1.57× | 147× | match |
| vm | 0.1 ms | 0.1 ms | 0.0 ms | 1.4 ms | 1.7 ms | 0.95× | 0.30× | 24× | 29× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 4.1 ms | 0.2 ms | 0.1 ms | 0.3 ms | 0.2 ms | 8.9 ms | 4.6 ms | 3.7 ms | 53.7 ms | 4.4 ms |
| tour | 39.1 ms | 0.3 ms | 0.1 ms | 0.4 ms | 1.0 ms | 44.9 ms | 4.7 ms | 4.6 ms | 56.9 ms | 6.4 ms |
| functions | 127.3 ms | 0.4 ms | 0.1 ms | 0.7 ms | 4.2 ms | 132.8 ms | 5.1 ms | 4.0 ms | 59.7 ms | 9.9 ms |
| errors | 34.5 ms | 0.3 ms | 0.1 ms | 0.4 ms | 1.0 ms | 38.6 ms | 4.9 ms | 4.4 ms | 54.7 ms | 6.0 ms |
| sieve | 12.2 ms | 0.3 ms | 0.1 ms | 0.4 ms | 0.4 ms | 17.9 ms | 5.2 ms | 5.0 ms | 61.4 ms | 89.8 ms |
| sorting | 107.1 ms | 0.3 ms | 0.1 ms | 0.6 ms | 2.7 ms | 286.8 ms | 179.9 ms | 106.0 ms | 339.4 ms | 34.8 s |
| life | 15.7 ms | 0.2 ms | 0.1 ms | 0.3 ms | 0.5 ms | 44.8 ms | 28.5 ms | 10.1 ms | 77.9 ms | 2,365.9 ms |
| maze | 37.8 ms | 0.3 ms | 0.1 ms | 0.4 ms | 1.0 ms | 42.5 ms | 4.9 ms | 4.3 ms | 52.1 ms | 7.9 ms |
| sudoku | 15.5 ms | 0.3 ms | 0.1 ms | 0.3 ms | 0.5 ms | 30.6 ms | 15.5 ms | 8.7 ms | 69.1 ms | 1,472.0 ms |
| vm | 28.9 ms | 0.3 ms | 0.1 ms | 0.3 ms | 0.7 ms | 33.5 ms | 4.7 ms | 4.2 ms | 50.8 ms | 6.6 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 0.2 ms | 0.0 ms | 0.0 ms | 0.01× | 0.12× | 0.3 ms | 0.1 ms | 0.2 ms | 53.7 ms | 3.7 ms | 4.4 ms | 0.07× | 0.08× |
| tour | 2.8 ms | 0.0 ms | 0.7 ms | 0.01× | 0.25× | 0.4 ms | 0.1 ms | 1.0 ms | 56.9 ms | 4.6 ms | 6.4 ms | 0.08× | 0.11× |
| functions | 2.6 ms | 0.0 ms | 0.7 ms | 0.01× | 0.23× | 0.7 ms | 0.1 ms | 4.2 ms | 59.7 ms | 4.0 ms | 9.9 ms | 0.07× | 0.15× |
| errors | 1.3 ms | 0.0 ms | 0.7 ms | 0.01× | 0.52× | 0.4 ms | 0.1 ms | 1.0 ms | 54.7 ms | 4.4 ms | 6.0 ms | 0.08× | 0.11× |
| sieve | 4.7 ms | 0.5 ms | 84.9 ms | 0.11× | 18× | 0.4 ms | 0.1 ms | 0.4 ms | 61.4 ms | 5.0 ms | 89.8 ms | 0.08× | 1.47× |
| sorting | 278.0 ms | 100.9 ms | 34.8 s | 0.36× | 129× | 0.6 ms | 0.1 ms | 2.7 ms | 339.4 ms | 106.0 ms | 34.8 s | 0.31× | 106× |
| life | 27.2 ms | 5.8 ms | 2,360.6 ms | 0.21× | 87× | 0.3 ms | 0.1 ms | 0.5 ms | 77.9 ms | 10.1 ms | 2,365.9 ms | 0.13× | 31× |
| maze | 1.5 ms | 0.0 ms | 3.0 ms | 0.02× | 2.07× | 0.4 ms | 0.1 ms | 1.0 ms | 52.1 ms | 4.3 ms | 7.9 ms | 0.08× | 0.15× |
| sudoku | 16.4 ms | 4.4 ms | 1,467.3 ms | 0.27× | 92× | 0.3 ms | 0.1 ms | 0.5 ms | 69.1 ms | 8.7 ms | 1,472.0 ms | 0.13× | 22× |
| vm | 1.4 ms | 0.0 ms | 1.7 ms | 0.01× | 1.22× | 0.3 ms | 0.1 ms | 0.7 ms | 50.8 ms | 4.2 ms | 6.6 ms | 0.08× | 0.12× |

### results/aot-037d32c/compiler.json

5 rounds, CPU 1 (AMD Ryzen 7 3800X 8-Core Processor, THP disabled). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-aot run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-aot / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| compile-squares | 0.7 ms | 0.8 ms | 0.7 ms | 59.2 ms | 22.8 ms | 1.23× | 1.04× | 89× | 34× | match |
| compile-vm | 5.1 ms | 5.2 ms | 3.5 ms | 163.5 ms | 461.6 ms | 0.99× | 0.70× | 32× | 90× | match |
| compile-sudoku | 4.0 ms | 4.1 ms | 2.7 ms | 159.5 ms | 352.2 ms | 0.97× | 0.68× | 38× | 86× | match |
| compile-functions | 22.5 ms | 22.4 ms | 13.7 ms | 273.7 ms | 2,081.2 ms | 0.99× | 0.62× | 12× | 91× | match |
| compile64-vm | 7.3 ms | 7.6 ms | 5.4 ms | 796.6 ms | 641.2 ms | 1.03× | 0.73× | 108× | 88× | match |
| compile64-functions | 32.0 ms | 32.0 ms | 22.6 ms | 1,062.0 ms | 2,765.0 ms | 1.00× | 0.70× | 33× | 86× | match |
| compile64-8g-vm | 7.3 ms | 7.5 ms | 5.5 ms | 283.5 ms | 629.5 ms | 1.03× | 0.74× | 39× | 87× | match |
| compile64-8g-functions | 33.1 ms | 32.6 ms | 22.7 ms | 426.5 ms | 2,860.5 ms | 1.00× | 0.69× | 13× | 86× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-aot prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-aot process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 3,752.5 ms | 3.0 ms | 0.1 ms | 33.8 ms | 99.6 ms | 3,765.2 ms | 10.9 ms | 5.6 ms | 186.0 ms | 135.5 ms |
| compile-vm | 3,548.7 ms | 3.0 ms | 0.1 ms | 35.2 ms | 99.7 ms | 3,568.6 ms | 15.7 ms | 8.0 ms | 320.0 ms | 573.5 ms |
| compile-sudoku | 3,697.1 ms | 3.1 ms | 0.1 ms | 36.5 ms | 102.2 ms | 3,713.9 ms | 14.6 ms | 7.7 ms | 318.4 ms | 468.3 ms |
| compile-functions | 3,607.9 ms | 2.9 ms | 0.1 ms | 35.7 ms | 101.3 ms | 3,643.5 ms | 32.9 ms | 19.3 ms | 423.0 ms | 2,199.0 ms |
| compile64-vm | 4,977.3 ms | 3.2 ms | 0.2 ms | 39.4 ms | 115.5 ms | 5,000.1 ms | 18.4 ms | 10.2 ms | 951.7 ms | 779.7 ms |
| compile64-functions | 5,032.6 ms | 3.2 ms | 0.2 ms | 38.4 ms | 116.3 ms | 5,080.8 ms | 42.8 ms | 27.7 ms | 1,207.3 ms | 2,898.2 ms |
| compile64-8g-vm | 4,941.1 ms | 3.2 ms | 0.2 ms | 39.2 ms | 115.2 ms | 4,963.8 ms | 17.7 ms | 10.0 ms | 436.8 ms | 760.3 ms |
| compile64-8g-functions | 5,067.0 ms | 3.3 ms | 0.2 ms | 39.7 ms | 116.5 ms | 5,117.3 ms | 44.3 ms | 28.0 ms | 587.6 ms | 2,994.7 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-aot run | wtime-decoded run | wtime-aot / V8 run | wtime-decoded / V8 run | V8 prep | wtime-aot prep | wtime-decoded prep | V8 process | wtime-aot process | wtime-decoded process | wtime-aot / V8 process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 59.2 ms | 0.7 ms | 22.8 ms | 0.01× | 0.38× | 33.8 ms | 0.1 ms | 99.6 ms | 186.0 ms | 5.6 ms | 135.5 ms | 0.03× | 0.72× |
| compile-vm | 163.5 ms | 3.5 ms | 461.6 ms | 0.02× | 2.84× | 35.2 ms | 0.1 ms | 99.7 ms | 320.0 ms | 8.0 ms | 573.5 ms | 0.02× | 1.79× |
| compile-sudoku | 159.5 ms | 2.7 ms | 352.2 ms | 0.02× | 2.21× | 36.5 ms | 0.1 ms | 102.2 ms | 318.4 ms | 7.7 ms | 468.3 ms | 0.02× | 1.48× |
| compile-functions | 273.7 ms | 13.7 ms | 2,081.2 ms | 0.05× | 7.59× | 35.7 ms | 0.1 ms | 101.3 ms | 423.0 ms | 19.3 ms | 2,199.0 ms | 0.05× | 5.18× |
| compile64-vm | 796.6 ms | 5.4 ms | 641.2 ms | 0.01× | 0.81× | 39.4 ms | 0.2 ms | 115.5 ms | 951.7 ms | 10.2 ms | 779.7 ms | 0.01× | 0.82× |
| compile64-functions | 1,062.0 ms | 22.6 ms | 2,765.0 ms | 0.02× | 2.60× | 38.4 ms | 0.2 ms | 116.3 ms | 1,207.3 ms | 27.7 ms | 2,898.2 ms | 0.02× | 2.40× |
| compile64-8g-vm | 283.5 ms | 5.5 ms | 629.5 ms | 0.02× | 2.22× | 39.2 ms | 0.2 ms | 115.2 ms | 436.8 ms | 10.0 ms | 760.3 ms | 0.02× | 1.75× |
| compile64-8g-functions | 426.5 ms | 22.7 ms | 2,860.5 ms | 0.05× | 6.78× | 39.7 ms | 0.2 ms | 116.5 ms | 587.6 ms | 28.0 ms | 2,994.7 ms | 0.05× | 5.14× |

