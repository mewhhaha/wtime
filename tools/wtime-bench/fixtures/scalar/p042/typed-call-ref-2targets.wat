(module
  (type $unary (func (param i32) (result i32)))
  (func $step_a (type $unary) (param $x i32) (result i32)
    (i32.add (local.get $x) (i32.const 1)))
  (func $step_b (type $unary) (param $x i32) (result i32)
    (i32.sub (local.get $x) (i32.const -1)))
  (elem declare func $step_a $step_b)
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $acc i32)
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $acc
          (call_ref $unary (local.get $acc)
            (if (result (ref $unary))
              (i32.eqz (i32.and (local.get $i) (i32.const 1)))
              (then (ref.func $step_a))
              (else (ref.func $step_b)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $acc)))
