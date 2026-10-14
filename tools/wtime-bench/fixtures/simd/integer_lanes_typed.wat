(module
  (func (export "run") (param $n i32) (param $initial v128) (result i32)
    (local $i i32) (local $v v128)
    (local.set $v (local.get $initial))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $v
          (v128.xor
            (i32x4.mul
              (i32x4.add (local.get $v) (v128.const i32x4 1 -3 5 6))
              (v128.const i32x4 3 5 7 9))
            (v128.const i32x4 324508639 -610839777 1431655765 -1431655766)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (i32.add
      (i32.add (i32x4.extract_lane 0 (local.get $v)) (i32x4.extract_lane 1 (local.get $v)))
      (i32.add (i32x4.extract_lane 2 (local.get $v)) (i32x4.extract_lane 3 (local.get $v))))))
