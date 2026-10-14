(module
  (memory (export "memory") 1 1)
  (func (export "run") (param $count i32) (result i32)
    (local $i i32) (local $sum i32)
    ;; Setup is intentionally inside run because the P009 driver invokes run directly.
    (block $init_done
      (loop $init
        (br_if $init_done (i32.ge_u (local.get $i) (local.get $count)))
        (i32.store8 (local.get $i)
          (i32.add (i32.mul (local.get $i) (i32.const 13)) (i32.const 7)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $init)))
    (local.set $i (i32.const 0))
    ;; The measured scan phase only reads guest memory.
    (block $scan_done
      (loop $scan
        (br_if $scan_done (i32.ge_u (local.get $i) (local.get $count)))
        (local.set $sum (i32.add (local.get $sum) (i32.load8_u (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $scan)))
    (local.get $sum)))
