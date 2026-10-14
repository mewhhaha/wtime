(module
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $v v128)
    (local.set $v (v128.const f64x2 1.25 -2.5))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $v (f64x2.add (local.get $v) (v128.const f64x2 0.5 -0.25)))
        (local.set $v (f64x2.mul (local.get $v) (v128.const f64x2 1.0 1.0)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (i32.add
      (i32.trunc_f64_s (f64x2.extract_lane 0 (local.get $v)))
      (i32.trunc_f64_s (f64x2.extract_lane 1 (local.get $v))))))
