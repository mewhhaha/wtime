(module
  (func (export "run") (param $x f64) (result i32)
    (if (result i32) (f64.ne (local.get $x) (local.get $x))
      (then (i32.const -1))
      (else
        (i32.wrap_i64
          (i64.shr_u
            (i64.reinterpret_f64 (f64.min (local.get $x) (f64.const 0.0)))
            (i64.const 63)))))))
