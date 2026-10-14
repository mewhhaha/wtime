(module
  (import "env" "memory" (memory 1 1 shared))
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $value i32)
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $value (i32.load (i32.const 0)))
        (i32.store (i32.const 0) (i32.add (local.get $value) (i32.const 1)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $i)))
