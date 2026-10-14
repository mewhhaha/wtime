(module
  (func $walk (param $n i32) (param $acc i32) (result i32)
    (if (result i32) (i32.eqz (local.get $n))
      (then (local.get $acc))
      (else
        (return_call $walk
          (i32.sub (local.get $n) (i32.const 1))
          (i32.add (i32.mul (local.get $acc) (i32.const 3)) (i32.const 1))))))
  (func (export "run") (param $n i32) (result i32)
    (call $walk (local.get $n) (i32.const 1))))
