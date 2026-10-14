(module
  (func (export "run") (param $case i32) (result i32)
    (if (result i32) (i32.eq (local.get $case) (i32.const 0))
      (then
        (i32.reinterpret_f32 (f32.min (f32.const -0.0) (f32.const 0.0))))
      (else
        (if (result i32) (i32.eq (local.get $case) (i32.const 1))
          (then
            (i32.reinterpret_f32 (f32.max (f32.const -0.0) (f32.const 0.0))))
          (else
            (if (result i32) (i32.eq (local.get $case) (i32.const 2))
              (then (f32.ne (f32.min (f32.const nan) (f32.const 1.0))
                            (f32.min (f32.const nan) (f32.const 1.0))))
              (else
                (if (result i32) (i32.eq (local.get $case) (i32.const 3))
                  (then (i32.trunc_sat_f32_s (f32.const inf)))
                  (else (i32.trunc_sat_f32_s (f32.const nan))))))))))))
