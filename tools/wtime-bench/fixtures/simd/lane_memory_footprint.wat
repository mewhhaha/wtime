(module
  (memory (export "memory") 1)
  (func (export "run") (param $vectors i32) (result i32)
    (local $i i32) (local $address i32) (local $v v128) (local $sum i32)
    ;; Initialization is part of run and therefore included in the timed call.
    (block $init_done
      (loop $init
        (br_if $init_done (i32.ge_u (local.get $i) (local.get $vectors)))
        (local.set $address (i32.mul (local.get $i) (i32.const 16)))
        (v128.store (local.get $address) (v128.const i32x4 1 2 3 4))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $init)))
    (local.set $i (i32.const 0))
    (block $update_done
      (loop $update
        (br_if $update_done (i32.ge_u (local.get $i) (local.get $vectors)))
        (local.set $address (i32.mul (local.get $i) (i32.const 16)))
        (local.set $v (i32x4.add
          (v128.load (local.get $address)) (v128.const i32x4 1 2 3 4)))
        (v128.store (local.get $address) (local.get $v))
        (local.set $sum (i32.add (local.get $sum) (i32.const 20)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $update)))
    (local.get $sum)))
