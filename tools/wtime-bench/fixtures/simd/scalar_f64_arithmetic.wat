(module
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $a f64) (local $b f64)
    (local.set $a (f64.const 1.25))
    (local.set $b (f64.const -2.5))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $a (f64.mul (f64.add (local.get $a) (f64.const 0.5)) (f64.const 1.0)))
        (local.set $b (f64.mul (f64.add (local.get $b) (f64.const -0.25)) (f64.const 1.0)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (i32.add (i32.trunc_f64_s (local.get $a))
             (i32.trunc_f64_s (local.get $b)))))
