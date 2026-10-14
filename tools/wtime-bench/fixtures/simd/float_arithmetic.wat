(module
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $v v128)
    (local.set $v (v128.const f32x4 1.25 -2.5 3.75 -4.0))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $v
          (f32x4.mul
            (f32x4.add (local.get $v) (v128.const f32x4 0.5 -0.25 0.125 1.0))
            (v128.const f32x4 1.0 1.0 1.0 1.0)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (i32.add
      (i32.add
        (i32.trunc_sat_f32_s (f32x4.extract_lane 0 (local.get $v)))
        (i32.trunc_sat_f32_s (f32x4.extract_lane 1 (local.get $v))))
      (i32.add
        (i32.trunc_sat_f32_s (f32x4.extract_lane 2 (local.get $v)))
        (i32.trunc_sat_f32_s (f32x4.extract_lane 3 (local.get $v)))))))
