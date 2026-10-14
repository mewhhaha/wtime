(module (memory 1 1)
 (func (export "run") (param $n i32) (result i32) (local $i i32) (local $acc i32)
 (block $done (loop $fill
  (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
  (i32.store (i32.mul (local.get $i) (i32.const 4)) (i32.mul (local.get $i) (i32.const 3)))
  (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $fill)))
 (local.set $i (i32.const 0))
 (block $done (loop $scan
  (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
  (local.set $acc (i32.add (local.get $acc) (i32.load (i32.mul (local.get $i) (i32.const 4)))))
  (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $scan))) (local.get $acc)))
