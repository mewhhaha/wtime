(module
 (func $step (param $x i32) (result i32)
  (if (result i32) (i32.eqz (i32.and (local.get $x) (i32.const 1)))
   (then (i32.add (i32.mul (local.get $x) (i32.const 3)) (i32.const 1)))
   (else (i32.xor (local.get $x) (i32.const 21845)))))
 (func (export "run") (param $n i32) (result i32) (local $i i32) (local $acc i32)
 (local.set $acc (i32.const 1))
 (block $done (loop $loop
  (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
  (local.set $acc (call $step (i32.xor (local.get $acc) (local.get $i))))
  (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $loop))) (local.get $acc)))
