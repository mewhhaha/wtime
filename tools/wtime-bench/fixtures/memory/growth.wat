(module
  (memory (export "memory") 1 17)
  (func (export "run") (param $delta i32) (result i32)
    (local $old i32)
    (local.set $old (memory.grow (local.get $delta)))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1))
      (then (local.get $old))
      (else (memory.size))))
  (func (export "size") (result i32) (memory.size)))
