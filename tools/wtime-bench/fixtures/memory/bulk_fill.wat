(module
  (memory (export "memory") 17 17)
  (func (export "run") (param $count i32) (result i32)
    (local $i i32) (local $sum i32)
    (memory.fill (i32.const 32) (i32.const 90) (local.get $count))
    (block $done
      (loop $sum
        (br_if $done (i32.ge_u (local.get $i) (local.get $count)))
        (local.set $sum (i32.add (local.get $sum) (i32.load8_u (i32.add (local.get $i) (i32.const 32)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $sum)))
    (local.get $sum)))
