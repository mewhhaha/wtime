### results/fa2128c/examples.json

3 rounds, CPU 1 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wtime-decoded run | wtime-before-decoded run | wtime-decoded / Wasmtime run | wtime-before-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.0 ms | 0.0 ms | 1.09× | 0.92× | match |
| tour | 0.1 ms | 1.0 ms | 0.6 ms | 9.53× | 5.99× | match |
| functions | 0.1 ms | 0.6 ms | 0.6 ms | 8.70× | 8.57× | match |
| errors | 0.1 ms | 0.6 ms | 0.5 ms | 13× | 9.48× | match |
| sieve | 0.7 ms | 84.3 ms | 129.5 ms | 112× | 172× | match |
| sorting | 299.2 ms | 27.0 s | 49.6 s | 104× | 166× | match |
| life | 22.6 ms | 2,692.1 ms | 3,740.9 ms | 119× | 166× | match |
| maze | 0.1 ms | 2.7 ms | 3.0 ms | 31× | 38× | match |
| sudoku | 13.4 ms | 1,780.6 ms | 2,076.7 ms | 123× | 150× | match |
| vm | 0.1 ms | 2.1 ms | 1.8 ms | 30× | 29× | match |

| Case | Wasmtime prep | wtime-decoded prep | wtime-before-decoded prep | Wasmtime process | wtime-decoded process | wtime-before-decoded process |
|---|---:|---:|---:|---:|---:|---:|
| squares | 5.8 ms | 0.2 ms | 0.2 ms | 11.6 ms | 4.1 ms | 4.6 ms |
| tour | 42.1 ms | 1.3 ms | 1.5 ms | 47.4 ms | 7.0 ms | 6.4 ms |
| functions | 140.4 ms | 3.6 ms | 4.9 ms | 145.1 ms | 9.2 ms | 9.8 ms |
| errors | 37.1 ms | 0.9 ms | 1.3 ms | 42.7 ms | 6.0 ms | 6.8 ms |
| sieve | 14.2 ms | 0.4 ms | 0.7 ms | 20.5 ms | 89.9 ms | 135.7 ms |
| sorting | 163.8 ms | 4.0 ms | 5.9 ms | 460.7 ms | 27.0 s | 49.6 s |
| life | 18.3 ms | 0.6 ms | 0.9 ms | 45.1 ms | 2,697.3 ms | 3,747.1 ms |
| maze | 41.5 ms | 1.3 ms | 1.6 ms | 47.0 ms | 8.3 ms | 9.0 ms |
| sudoku | 23.9 ms | 0.7 ms | 0.8 ms | 42.5 ms | 1,786.4 ms | 2,082.8 ms |
| vm | 35.3 ms | 0.9 ms | 1.4 ms | 40.7 ms | 7.9 ms | 7.9 ms |

### results/fa2128c/compiler.json

3 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---|
| compile-squares | 1.3 ms | 1.4 ms | 42.3 ms | 1.03× | 31× | match |
| compile-vm | 6.9 ms | 6.5 ms | 601.0 ms | 1.00× | 79× | match |
| compile-sudoku | 4.8 ms | 4.9 ms | 287.9 ms | 1.01× | 63× | match |
| compile-functions | 28.6 ms | 27.9 ms | 2,110.1 ms | 0.93× | 68× | match |
| compile64-vm | 13.9 ms | 8.8 ms | 693.1 ms | 0.73× | 50× | match |
| compile64-functions | 35.1 ms | 37.1 ms | 3,274.0 ms | 1.00× | 87× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|
| compile-squares | 7,506.4 ms | 7.3 ms | 261.4 ms | 7,522.5 ms | 20.0 ms | 326.4 ms |
| compile-vm | 6,280.9 ms | 6.3 ms | 251.3 ms | 6,305.0 ms | 22.7 ms | 872.0 ms |
| compile-sudoku | 6,885.9 ms | 6.2 ms | 200.1 ms | 6,906.4 ms | 22.1 ms | 502.2 ms |
| compile-functions | 6,560.3 ms | 6.4 ms | 257.3 ms | 6,618.4 ms | 44.9 ms | 2,414.3 ms |
| compile64-vm | 11.4 s | 7.4 ms | 251.2 ms | 11.4 s | 26.8 ms | 962.9 ms |
| compile64-functions | 11.3 s | 7.0 ms | 260.5 ms | 11.4 s | 55.0 ms | 3,561.6 ms |

