(module
  (type $unary (func (param i32) (result i32)))
  (table 3 funcref)
  (func $step_a (type $unary) (param $x i32) (result i32)
    (i32.add (local.get $x) (i32.const 1)))
  (func $step_b (type $unary) (param $x i32) (result i32)
    (i32.sub (local.get $x) (i32.const -1)))
  (func $step_c (type $unary) (param $x i32) (result i32)
    (i32.add (i32.mul (local.get $x) (i32.const 1)) (i32.const 1)))
  (elem (i32.const 0) $step_a $step_b $step_c)
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $acc i32)
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $acc
          (call_indirect (type $unary) (local.get $acc)
            (i32.rem_u (local.get $i) (i32.const 3))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $acc)))
