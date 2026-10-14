(module
  (type $unary (func (param i32) (result i32)))
  (table 1 funcref)
  (global $next (mut i32) (i32.const 0))
  (func $step_a (type $unary) (param $x i32) (result i32)
    (i32.add (local.get $x) (i32.const 1)))
  (func $step_b (type $unary) (param $x i32) (result i32)
    (i32.sub (local.get $x) (i32.const -1)))
  (elem declare func $step_a $step_b)
  (elem (i32.const 0) $step_a)
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $acc i32)
    (if (global.get $next)
      (then (table.set (i32.const 0) (ref.func $step_b)))
      (else (table.set (i32.const 0) (ref.func $step_a))))
    (global.set $next (i32.eqz (global.get $next)))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $acc
          (call_indirect (type $unary) (local.get $acc) (i32.const 0)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $acc)))
