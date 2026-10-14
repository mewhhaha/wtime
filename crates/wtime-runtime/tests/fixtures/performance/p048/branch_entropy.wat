(module
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $state i32)
    (local.set $state (i32.const 1))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $state
          (i32.add (i32.mul (local.get $state) (i32.const 1664525)) (i32.const 1013904223)))
        (if (i32.ne (i32.and (local.get $state) (i32.const -2147483648)) (i32.const 0))
          (then (local.set $state (i32.xor (local.get $state) (i32.const 1511506142))))
          (else (local.set $state (i32.add (local.get $state) (i32.const 265443576)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $state)))
