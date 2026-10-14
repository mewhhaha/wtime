(module
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $a f32) (local $b f32) (local $c f32) (local $d f32)
    (local.set $a (f32.const 1.25))
    (local.set $b (f32.const -2.5))
    (local.set $c (f32.const 3.75))
    (local.set $d (f32.const -4.0))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $a (f32.mul (f32.add (local.get $a) (f32.const 0.5)) (f32.const 1.0)))
        (local.set $b (f32.mul (f32.add (local.get $b) (f32.const -0.25)) (f32.const 1.0)))
        (local.set $c (f32.mul (f32.add (local.get $c) (f32.const 0.125)) (f32.const 1.0)))
        (local.set $d (f32.mul (f32.add (local.get $d) (f32.const 1.0)) (f32.const 1.0)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (i32.add
      (i32.add (i32.trunc_sat_f32_s (local.get $a)) (i32.trunc_sat_f32_s (local.get $b)))
      (i32.add (i32.trunc_sat_f32_s (local.get $c)) (i32.trunc_sat_f32_s (local.get $d))))))
