(module
  (memory (export "memory") 1)
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $v v128)
    (v128.store (i32.const 0) (v128.const i32x4 1 2 3 4))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $v (v128.load (i32.const 0)))
        (local.set $v (i32x4.add (local.get $v) (v128.const i32x4 1 2 3 4)))
        (v128.store (i32.const 0) (local.get $v))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.set $v (v128.load (i32.const 0)))
    (i32.add
      (i32.add (i32x4.extract_lane 0 (local.get $v)) (i32x4.extract_lane 1 (local.get $v)))
      (i32.add (i32x4.extract_lane 2 (local.get $v)) (i32x4.extract_lane 3 (local.get $v))))))
