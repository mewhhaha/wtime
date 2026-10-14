(module (func (export "run") (param $n i32) (result i32)
 (local $i i32) (local $acc i32) (local.set $acc (i32.const 1))
 (block $done (loop $loop
  (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
  (local.set $acc (i32.add (i32.mul (local.get $acc) (i32.const 1664525)) (i32.const 1013904223)))
  (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $loop))) (local.get $acc)))
