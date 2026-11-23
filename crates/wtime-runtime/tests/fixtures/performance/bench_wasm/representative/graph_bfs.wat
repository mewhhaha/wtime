;; Original benchmark fixture, authored for wtime; SPDX-License-Identifier: CC0-1.0.
(module
  (memory (export "memory") 1 1)
  (func $edge (param $u i32) (param $v i32) (param $n i32) (result i32)
    (local $next1 i32) (local $next3 i32)
    (local.set $next1 (i32.rem_u (i32.add (local.get $u) (i32.const 1)) (local.get $n)))
    (local.set $next3 (i32.rem_u (i32.add (local.get $u) (i32.const 3)) (local.get $n)))
    (i32.or
      (i32.eq (local.get $v) (local.get $next1))
      (i32.eq (local.get $v) (local.get $next3))))
  (func (export "prepare") (param $n i32)
    (local $i i32)
    (block $clear_done
      (loop $clear
        (br_if $clear_done (i32.ge_u (local.get $i) (local.get $n)))
        (i32.store8 (local.get $i) (i32.const 0))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $clear))))
  (func (export "run") (param $n i32) (result i32)
    (local $head i32) (local $tail i32)
    (local $node i32) (local $neighbor i32) (local $hash i32)
    (i32.store8 (i32.const 0) (i32.const 1))
    (i32.store (i32.const 128) (i32.const 0))
    (local.set $tail (i32.const 1))
    (local.set $hash (i32.const -2128831035))
    (block $bfs_done
      (loop $bfs
        (br_if $bfs_done (i32.ge_u (local.get $head) (local.get $tail)))
        (local.set $node (i32.load (i32.add (i32.const 128)
                                            (i32.mul (local.get $head) (i32.const 4)))))
        (local.set $hash
          (i32.mul (i32.xor (local.get $hash) (local.get $node)) (i32.const 16777619)))
        (i32.store8 (i32.add (i32.const 4096) (local.get $head)) (local.get $node))
        (local.set $neighbor (i32.const 0))
        (block $scan_done
          (loop $scan
            (br_if $scan_done (i32.ge_u (local.get $neighbor) (local.get $n)))
            (if (call $edge (local.get $node) (local.get $neighbor) (local.get $n))
              (then
                (if (i32.eqz (i32.load8_u (local.get $neighbor)))
                  (then
                    (i32.store8 (local.get $neighbor) (i32.const 1))
                    (i32.store
                      (i32.add (i32.const 128)
                               (i32.mul (local.get $tail) (i32.const 4)))
                      (local.get $neighbor))
                    (local.set $tail (i32.add (local.get $tail) (i32.const 1)))))))
            (local.set $neighbor (i32.add (local.get $neighbor) (i32.const 1)))
            (br $scan)))
        (local.set $head (i32.add (local.get $head) (i32.const 1)))
        (br $bfs)))
    (local.get $hash)))
