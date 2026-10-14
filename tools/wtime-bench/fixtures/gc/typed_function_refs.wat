(module
  (type $step_type (func (param i32) (result i32)))
  (elem declare func $step)
  (func $step (type $step_type) (param $value i32) (result i32)
    (i32.add (local.get $value) (i32.const 1)))
  (func (export "run") (param $count i32) (result i32)
    (local $index i32)
    (local $sum i32)
    (block $done
      (loop $calls
        (br_if $done (i32.ge_u (local.get $index) (local.get $count)))
        (local.set $sum
          (i32.add (local.get $sum)
            (call_ref $step_type (local.get $index) (ref.func $step))))
        (local.set $index (i32.add (local.get $index) (i32.const 1)))
        (br $calls)))
    (local.get $sum)))
