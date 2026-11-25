;; Original benchmark fixture, authored for wtime; SPDX-License-Identifier: CC0-1.0.
;; In-place binary heapsort over N i32 keys (branch-heavy, data-dependent scalar indexing).
;; Memory layout (1 page = 64 KiB, supports n <= 8192):
;;   source backup: [0 .. 32768), working array: [32768 .. 65536)
(module
  (memory (export "memory") 1 1)

  (func (export "prepare") (param $n i32)
    (local $i i32)
    (local $x i32)
    (local.set $x (i32.const 0x2468ace1))
    (local.set $i (i32.const 0))
    (block $done
      (loop $fill
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $x (i32.xor (local.get $x) (i32.shl (local.get $x) (i32.const 13))))
        (local.set $x (i32.xor (local.get $x) (i32.shr_u (local.get $x) (i32.const 17))))
        (local.set $x (i32.xor (local.get $x) (i32.shl (local.get $x) (i32.const 5))))
        (i32.store (i32.shl (local.get $i) (i32.const 2)) (local.get $x))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $fill))))

  (func $sift_down (param $root i32) (param $end i32)
    (local $cur i32)
    (local $child i32)
    (local $swap i32)
    (local $v_swap i32)
    (local $v_child i32)
    (local $v_right i32)
    (local $tmp i32)
    (local.set $cur (local.get $root))
    (block $done
      (loop $sift
        (local.set $child (i32.add (i32.shl (local.get $cur) (i32.const 1)) (i32.const 1)))
        (br_if $done (i32.ge_u (local.get $child) (local.get $end)))
        (local.set $swap (local.get $cur))
        (local.set $v_swap
          (i32.load offset=32768 (i32.shl (local.get $cur) (i32.const 2))))
        (local.set $v_child
          (i32.load offset=32768 (i32.shl (local.get $child) (i32.const 2))))
        (if (i32.lt_s (local.get $v_swap) (local.get $v_child))
          (then
            (local.set $swap (local.get $child))
            (local.set $v_swap (local.get $v_child))))
        (if (i32.lt_u (i32.add (local.get $child) (i32.const 1)) (local.get $end))
          (then
            (local.set $v_right
              (i32.load offset=32768 (i32.shl (i32.add (local.get $child) (i32.const 1)) (i32.const 2))))
            (if (i32.lt_s (local.get $v_swap) (local.get $v_right))
              (then
                (local.set $swap (i32.add (local.get $child) (i32.const 1)))
                (local.set $v_swap (local.get $v_right))))))
        (br_if $done (i32.eq (local.get $swap) (local.get $cur)))
        (local.set $tmp (i32.load offset=32768 (i32.shl (local.get $cur) (i32.const 2))))
        (i32.store offset=32768 (i32.shl (local.get $cur) (i32.const 2)) (local.get $v_swap))
        (i32.store offset=32768 (i32.shl (local.get $swap) (i32.const 2)) (local.get $tmp))
        (local.set $cur (local.get $swap))
        (br $sift))))

  (func (export "run") (param $n i32) (result i32)
    (local $i i32)
    (local $end i32)
    (local $tmp i32)
    (local $h i32)
    (local $prev i32)
    (local $cur i32)
    (memory.copy (i32.const 32768) (i32.const 0) (i32.shl (local.get $n) (i32.const 2)))
    (if (i32.gt_u (local.get $n) (i32.const 1))
      (then
        ;; Build max-heap
        (local.set $i (i32.shr_u (local.get $n) (i32.const 1)))
        (block $build_done
          (loop $build
            (br_if $build_done (i32.eqz (local.get $i)))
            (local.set $i (i32.sub (local.get $i) (i32.const 1)))
            (call $sift_down (local.get $i) (local.get $n))
            (br $build)))
        ;; Extract max repeatedly
        (local.set $end (local.get $n))
        (block $sort_done
          (loop $sort
            (local.set $end (i32.sub (local.get $end) (i32.const 1)))
            (br_if $sort_done (i32.eqz (local.get $end)))
            (local.set $tmp (i32.load offset=32768 (i32.const 0)))
            (i32.store offset=32768 (i32.const 0)
              (i32.load offset=32768 (i32.shl (local.get $end) (i32.const 2))))
            (i32.store offset=32768 (i32.shl (local.get $end) (i32.const 2)) (local.get $tmp))
            (call $sift_down (i32.const 0) (local.get $end))
            (br $sort)))))
    ;; Verify sorted order & hash output
    (local.set $h (i32.const 0x811c9dc5))
    (local.set $prev (i32.const -2147483648))
    (local.set $i (i32.const 0))
    (block $h_done
      (loop $h_loop
        (br_if $h_done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $cur (i32.load offset=32768 (i32.shl (local.get $i) (i32.const 2))))
        (if (i32.lt_s (local.get $cur) (local.get $prev))
          (then (unreachable)))
        (local.set $prev (local.get $cur))
        (local.set $h (i32.mul (i32.xor (local.get $h) (local.get $cur)) (i32.const 16777619)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $h_loop)))
    (local.get $h)))
