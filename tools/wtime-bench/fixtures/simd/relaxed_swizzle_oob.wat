(module
  (func (export "run") (param $index i32) (result i32)
    (local $v v128)
    (local.set $v
      (i8x16.relaxed_swizzle
        (v128.const i8x16 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1)
        (i8x16.splat (local.get $index))))
    (local.set $v (i16x8.extadd_pairwise_i8x16_u (local.get $v)))
    (local.set $v (i32x4.extadd_pairwise_i16x8_u (local.get $v)))
    (i32.add
      (i32.add (i32x4.extract_lane 0 (local.get $v)) (i32x4.extract_lane 1 (local.get $v)))
      (i32.add (i32x4.extract_lane 2 (local.get $v)) (i32x4.extract_lane 3 (local.get $v))))))
