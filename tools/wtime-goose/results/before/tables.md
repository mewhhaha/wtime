### results/examples.json

3 rounds, CPU 1 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wtime-decoded run | wtime-compact run | wtime-register-adaptive=3 run | wtime-register run | wtime-decoded / Wasmtime run | wtime-compact / Wasmtime run | wtime-register-adaptive=3 / Wasmtime run | wtime-register / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| squares | 0.0 ms | 0.1 ms | 0.1 ms | 0.1 ms | 0.1 ms | 2.57× | 2.42× | 2.07× | 2.15× | match |
| tour | 0.1 ms | 12.3 ms | 10.8 ms | 10.9 ms | 11.0 ms | 92× | 81× | 82× | 82× | match |
| functions | 0.1 ms | 11.1 ms | 11.0 ms | 10.8 ms | 10.7 ms | 116× | 113× | 112× | 112× | match |
| errors | 0.1 ms | 10.7 ms | 10.7 ms | 10.7 ms | 10.6 ms | 163× | 163× | 166× | 165× | match |
| sieve | 0.6 ms | 687.6 ms | 697.3 ms | 803.5 ms | 834.5 ms | 1,026× | 982× | 1,217× | 1,288× | match |
| sorting | 144.6 ms | 302.4 s | 305.2 s | 305.3 s | 302.9 s | 2,091× | 2,109× | 2,111× | 2,094× | match |
| life | 21.9 ms | 18.5 s | 19.5 s | 18.4 s | 17.8 s | 851× | 896× | 846× | 806× | match |
| maze | 0.1 ms | 28.2 ms | 27.6 ms | 27.7 ms | 27.0 ms | 291× | 281× | 287× | 278× | match |
| sudoku | 12.4 ms | 8,978.3 ms | 9,040.1 ms | 9,525.7 ms | 9,586.1 ms | 749× | 673× | 774× | 729× | match |
| vm | 0.1 ms | 14.9 ms | 14.9 ms | 15.4 ms | 15.7 ms | 191× | 192× | 197× | 200× | match |

| Case | Wasmtime prep | wtime-decoded prep | wtime-compact prep | wtime-register-adaptive=3 prep | wtime-register prep | Wasmtime process | wtime-decoded process | wtime-compact process | wtime-register-adaptive=3 process | wtime-register process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| squares | 4.3 ms | 0.6 ms | 0.6 ms | 0.6 ms | 0.6 ms | 8.9 ms | 3.7 ms | 3.6 ms | 3.6 ms | 3.5 ms |
| tour | 60.9 ms | 4.1 ms | 4.1 ms | 3.9 ms | 3.9 ms | 67.6 ms | 21.9 ms | 19.8 ms | 19.3 ms | 19.8 ms |
| functions | 190.3 ms | 12.1 ms | 11.5 ms | 11.5 ms | 11.4 ms | 195.0 ms | 29.1 ms | 28.2 ms | 27.8 ms | 27.8 ms |
| errors | 49.4 ms | 3.7 ms | 3.6 ms | 3.5 ms | 3.5 ms | 54.0 ms | 19.6 ms | 19.4 ms | 19.2 ms | 18.9 ms |
| sieve | 11.7 ms | 1.3 ms | 2.0 ms | 1.6 ms | 1.3 ms | 15.7 ms | 693.6 ms | 703.3 ms | 809.8 ms | 842.5 ms |
| sorting | 125.4 ms | 12.4 ms | 9.5 ms | 9.3 ms | 9.1 ms | 274.7 ms | 302.4 s | 305.2 s | 305.3 s | 302.9 s |
| life | 17.7 ms | 1.7 ms | 2.2 ms | 2.2 ms | 1.6 ms | 43.3 ms | 18.5 s | 19.5 s | 18.4 s | 17.8 s |
| maze | 58.0 ms | 4.1 ms | 4.4 ms | 4.0 ms | 4.2 ms | 63.2 ms | 37.7 ms | 37.8 ms | 36.6 ms | 36.5 ms |
| sudoku | 21.8 ms | 1.7 ms | 2.0 ms | 1.8 ms | 1.5 ms | 40.4 ms | 8,985.5 ms | 9,047.1 ms | 9,532.3 ms | 9,593.2 ms |
| vm | 41.5 ms | 3.0 ms | 2.9 ms | 2.9 ms | 3.0 ms | 46.0 ms | 22.8 ms | 22.5 ms | 22.5 ms | 23.2 ms |

### results/compiler.json

3 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wtime-decoded run | wtime-compact run | wtime-register-adaptive=3 run | wtime-register run | wtime-decoded / Wasmtime run | wtime-compact / Wasmtime run | wtime-register-adaptive=3 / Wasmtime run | wtime-register / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| compile-squares | 1.5 ms | 142.9 ms | 144.5 ms | 160.6 ms | 158.2 ms | 99× | 87× | 110× | 109× | match |
| compile-vm | 8.3 ms | 2,916.6 ms | 3,073.1 ms | 2,658.4 ms | 2,983.9 ms | 354× | 308× | 306× | 359× | match |
| compile-sudoku | 5.2 ms | 2,137.3 ms | 2,044.9 ms | 2,036.9 ms | 2,087.3 ms | 410× | 392× | 390× | 400× | match |
| compile-functions | 28.2 ms | 12.7 s | 13.0 s | 12.9 s | 13.1 s | 449× | 491× | 469× | 466× | match |

| Case | Wasmtime prep | wtime-decoded prep | wtime-compact prep | wtime-register-adaptive=3 prep | wtime-register prep | Wasmtime process | wtime-decoded process | wtime-compact process | wtime-register-adaptive=3 process | wtime-register process |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| compile-squares | 6,460.9 ms | 3,096.3 ms | 3,311.9 ms | 3,194.0 ms | 2,904.7 ms | 6,480.3 ms | 3,512.0 ms | 3,705.3 ms | 3,600.4 ms | 3,262.6 ms |
| compile-vm | 6,930.5 ms | 3,304.3 ms | 2,855.9 ms | 2,741.7 ms | 2,722.8 ms | 6,963.7 ms | 6,459.4 ms | 5,971.7 ms | 5,595.7 ms | 5,957.7 ms |
| compile-sudoku | 6,869.3 ms | 3,083.1 ms | 2,821.8 ms | 2,840.9 ms | 2,788.4 ms | 6,890.3 ms | 5,467.5 ms | 5,125.7 ms | 5,160.2 ms | 5,090.0 ms |
| compile-functions | 6,458.0 ms | 2,930.0 ms | 2,829.3 ms | 2,742.8 ms | 2,802.6 ms | 6,511.5 ms | 16.2 s | 16.2 s | 15.8 s | 16.3 s |

### results/compiler-cached.json

5 rounds, CPU 2 (Intel(R) Xeon(R) Processor @ 2.10GHz). Medians of fresh processes; ratios are medians of per-round wtime/Wasmtime pairs.

| Case | Wasmtime run | wasmtime-cached run | wtime-decoded run | wasmtime-cached / Wasmtime run | wtime-decoded / Wasmtime run | outputs |
|---|---:|---:|---:|---:|---:|---|
| compile-squares | 1.7 ms | 1.9 ms | 137.1 ms | 0.98× | 71× | match |
| compile-vm | 6.2 ms | 6.3 ms | 3,014.7 ms | 1.02× | 468× | match |

| Case | Wasmtime prep | wasmtime-cached prep | wtime-decoded prep | Wasmtime process | wasmtime-cached process | wtime-decoded process |
|---|---:|---:|---:|---:|---:|---:|
| compile-squares | 7,064.3 ms | 7.7 ms | 3,110.8 ms | 7,096.3 ms | 18.7 ms | 3,554.1 ms |
| compile-vm | 6,886.7 ms | 7.4 ms | 3,039.4 ms | 6,914.8 ms | 21.7 ms | 6,265.3 ms |

