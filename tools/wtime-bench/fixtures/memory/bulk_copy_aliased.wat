(module
  (import "mem" "left" (memory $left 17 17))
  (import "mem" "right" (memory $right 17 17))
  (func (export "run") (param $count i32) (result i32)
    (local $i i32) (local $sum i32)
    (block $init_done
      (loop $init
        (br_if $init_done (i32.gt_u (local.get $i) (local.get $count)))
        (i32.store8 $left (local.get $i)
          (i32.add (i32.mul (local.get $i) (i32.const 29)) (i32.const 7)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $init)))
    (memory.copy $right $left (i32.const 1) (i32.const 0) (local.get $count))
    (local.set $i (i32.const 0))
    (block $sum_done
      (loop $sum
        (br_if $sum_done (i32.gt_u (local.get $i) (local.get $count)))
        (local.set $sum (i32.add (local.get $sum) (i32.load8_u $left (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $sum)))
    (local.get $sum)))
