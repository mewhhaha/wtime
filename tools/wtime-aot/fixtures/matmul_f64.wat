;; Original benchmark fixture, authored for wtime; SPDX-License-Identifier: CC0-1.0.
;; Dense NxN f64 matrix multiplication (C = A * B) using cache-friendly i-k-j AXPY inner loop.
;; Memory layout (4 pages = 256 KiB, supports n <= 64):
;;   A: [0 .. 32768), B: [32768 .. 65536), C: [65536 .. 98304)
(module
  (memory (export "memory") 4 4)

  (func (export "prepare") (param $n i32)
    (local $total i32)
    (local $i i32)
    (local $seed i32)
    (local $va f64)
    (local $vb f64)
    (local.set $total (i32.mul (local.get $n) (local.get $n)))
    (local.set $seed (i32.const 0x12345678))
    (local.set $i (i32.const 0))
    (block $done
      (loop $fill
        (br_if $done (i32.ge_u (local.get $i) (local.get $total)))
        (local.set $seed
          (i32.add (i32.mul (local.get $seed) (i32.const 1664525)) (i32.const 1013904223)))
        (local.set $va
          (f64.add
            (f64.mul (f64.convert_i32_u (i32.and (local.get $seed) (i32.const 65535)))
                     (f64.const 0.0000152587890625))
            (f64.const 0.125)))
        (f64.store (i32.shl (local.get $i) (i32.const 3)) (local.get $va))
        (local.set $seed
          (i32.add (i32.mul (local.get $seed) (i32.const 1664525)) (i32.const 1013904223)))
        (local.set $vb
          (f64.sub
            (f64.mul (f64.convert_i32_u (i32.and (local.get $seed) (i32.const 65535)))
                     (f64.const 0.0000152587890625))
            (f64.const 0.375)))
        (f64.store (i32.add (i32.const 32768) (i32.shl (local.get $i) (i32.const 3))) (local.get $vb))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $fill))))

  (func (export "run") (param $n i32) (result i32)
    (local $i i32)
    (local $k i32)
    (local $j i32)
    (local $total i32)
    (local $c_row i32)
    (local $b_row i32)
    (local $a_row i32)
    (local $aik f64)
    (local $hash i64)
    (local $bits i64)
    (local.set $total (i32.mul (local.get $n) (local.get $n)))
    (memory.fill (i32.const 65536) (i32.const 0) (i32.shl (local.get $total) (i32.const 3)))
    (local.set $i (i32.const 0))
    (block $i_done
      (loop $i_loop
        (br_if $i_done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $a_row (i32.shl (i32.mul (local.get $i) (local.get $n)) (i32.const 3)))
        (local.set $c_row (i32.add (i32.const 65536) (local.get $a_row)))
        (local.set $k (i32.const 0))
        (block $k_done
          (loop $k_loop
            (br_if $k_done (i32.ge_u (local.get $k) (local.get $n)))
            (local.set $aik
              (f64.load (i32.add (local.get $a_row) (i32.shl (local.get $k) (i32.const 3)))))
            (local.set $b_row
              (i32.add (i32.const 32768)
                (i32.shl (i32.mul (local.get $k) (local.get $n)) (i32.const 3))))
            (local.set $j (i32.const 0))
            (block $j_done
              (loop $j_loop
                (br_if $j_done (i32.ge_u (local.get $j) (local.get $n)))
                (f64.store
                  (i32.add (local.get $c_row) (i32.shl (local.get $j) (i32.const 3)))
                  (f64.add
                    (f64.load (i32.add (local.get $c_row) (i32.shl (local.get $j) (i32.const 3))))
                    (f64.mul
                      (local.get $aik)
                      (f64.load (i32.add (local.get $b_row) (i32.shl (local.get $j) (i32.const 3)))))))
                (local.set $j (i32.add (local.get $j) (i32.const 1)))
                (br $j_loop)))
            (local.set $k (i32.add (local.get $k) (i32.const 1)))
            (br $k_loop)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $i_loop)))
    (local.set $hash (i64.const 0xcbf29ce484222325))
    (local.set $i (i32.const 0))
    (block $h_done
      (loop $h_loop
        (br_if $h_done (i32.ge_u (local.get $i) (local.get $total)))
        (local.set $bits
          (i64.reinterpret_f64
            (f64.load (i32.add (i32.const 65536) (i32.shl (local.get $i) (i32.const 3))))))
        (local.set $hash
          (i64.mul (i64.xor (local.get $hash) (local.get $bits)) (i64.const 0x100000001b3)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $h_loop)))
    (i32.wrap_i64 (i64.xor (local.get $hash) (i64.shr_u (local.get $hash) (i64.const 32))))))
