(module
  (type $unary (func (param i32) (result i32)))
  (table 1 funcref)
  (func $step (type $unary) (param $x i32) (result i32)
    (i32.add (i32.mul (local.get $x) (i32.const 3)) (i32.const 1)))
  (elem (i32.const 0) $step)
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $acc i32)
    (local.set $acc (i32.const 1))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $acc (call_indirect (type $unary) (local.get $acc) (i32.const 0)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $acc)))
