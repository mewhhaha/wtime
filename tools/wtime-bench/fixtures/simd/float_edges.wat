(module
  (func (export "run") (param $case i32) (result i32)
    (local $v v128)
    (if (result i32) (i32.eq (local.get $case) (i32.const 0))
      (then
        (i32.reinterpret_f32
          (f32x4.extract_lane 0
            (f32x4.min
              (v128.const f32x4 -0.0 0.0 nan 1.0)
              (v128.const f32x4 0.0 -0.0 2.0 3.0)))))
      (else
        (if (result i32) (i32.eq (local.get $case) (i32.const 1))
          (then
            (i32.reinterpret_f32
              (f32x4.extract_lane 0
                (f32x4.max
                  (v128.const f32x4 -0.0 0.0 nan 1.0)
                  (v128.const f32x4 0.0 -0.0 2.0 3.0)))))
          (else
            (if (result i32) (i32.eq (local.get $case) (i32.const 2))
              (then
                (local.set $v
                  (f32x4.min
                    (v128.const f32x4 nan 1.0 2.0 3.0)
                    (v128.const f32x4 4.0 5.0 6.0 7.0)))
                (i32x4.bitmask (f32x4.ne (local.get $v) (local.get $v))))
              (else
                (if (result i32) (i32.eq (local.get $case) (i32.const 3))
                  (then (i32.trunc_sat_f32_s (f32.const inf)))
                  (else
                    (if (result i32) (i32.eq (local.get $case) (i32.const 4))
                      (then (i32.trunc_sat_f32_s (f32.const nan)))
                      (else (i32.const -1)))))))))))))
