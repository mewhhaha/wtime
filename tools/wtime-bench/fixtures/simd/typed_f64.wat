(module
  (func (export "run") (param $x f64) (result i32)
    (i32.trunc_f64_s (local.get $x))))
