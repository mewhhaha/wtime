### results/v8-dc33870/compiler.json

3 rounds, CPU 1-3 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | v8 run | wtime-decoded run | wasmtime-cached / Wasmtime run | v8 / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| compile-squares | 1.3 ms | 1.3 ms | 109.3 ms | 26.8 ms | 1.11× | 83× | 23× | match |
| compile-vm | 5.3 ms | 5.7 ms | 173.6 ms | 424.8 ms | 1.08× | 33× | 80× | match |
| compile-sudoku | 4.7 ms | 4.4 ms | 174.5 ms | 426.9 ms | 0.93× | 37× | 73× | match |
| compile-functions | 23.7 ms | 25.3 ms | 234.4 ms | 1,925.0 ms | 1.09× | 10× | 81× | match |
| compile64-vm | 8.8 ms | 11.0 ms | 609.5 ms | 581.8 ms | 1.26× | 69× | 66× | match |
| compile64-functions | 35.5 ms | 36.4 ms | 751.3 ms | 3,081.7 ms | 1.00× | 21× | 87× | match |

| Case | Wasmtime prep | wasmtime-cached prep | v8 prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | v8 process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 2,198.3 ms | 6.6 ms | 18.8 ms | 101.0 ms | 2,224.8 ms | 17.2 ms | 225.4 ms | 148.5 ms |
| compile-vm | 2,032.9 ms | 5.8 ms | 16.1 ms | 103.8 ms | 2,058.2 ms | 21.2 ms | 299.2 ms | 552.0 ms |
| compile-sudoku | 2,243.1 ms | 5.9 ms | 16.0 ms | 106.9 ms | 2,265.6 ms | 21.2 ms | 310.4 ms | 552.4 ms |
| compile-functions | 2,119.9 ms | 6.4 ms | 16.3 ms | 100.4 ms | 2,165.2 ms | 41.4 ms | 369.5 ms | 2,044.1 ms |
| compile64-vm | 3,507.8 ms | 8.0 ms | 20.6 ms | 130.4 ms | 3,543.2 ms | 30.2 ms | 762.4 ms | 720.9 ms |
| compile64-functions | 3,912.9 ms | 7.3 ms | 23.6 ms | 121.3 ms | 3,972.6 ms | 54.7 ms | 907.1 ms | 3,236.5 ms |

Against V8 (Deno): run, preparation and whole process; ratios are medians of per-round wtime/V8 pairs, below 1 when wtime is faster.

| Case | V8 run | wtime-decoded run | wtime-decoded / V8 run | V8 prep | wtime-decoded prep | V8 process | wtime-decoded process | wtime-decoded / V8 process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 109.3 ms | 26.8 ms | 0.25× | 18.8 ms | 101.0 ms | 225.4 ms | 148.5 ms | 0.65× |
| compile-vm | 173.6 ms | 424.8 ms | 2.37× | 16.1 ms | 103.8 ms | 299.2 ms | 552.0 ms | 1.77× |
| compile-sudoku | 174.5 ms | 426.9 ms | 2.45× | 16.0 ms | 106.9 ms | 310.4 ms | 552.4 ms | 1.88× |
| compile-functions | 234.4 ms | 1,925.0 ms | 7.79× | 16.3 ms | 100.4 ms | 369.5 ms | 2,044.1 ms | 5.53× |
| compile64-vm | 609.5 ms | 581.8 ms | 1.03× | 20.6 ms | 130.4 ms | 762.4 ms | 720.9 ms | 1.07× |
| compile64-functions | 751.3 ms | 3,081.7 ms | 3.89× | 23.6 ms | 121.3 ms | 907.1 ms | 3,236.5 ms | 3.44× |

