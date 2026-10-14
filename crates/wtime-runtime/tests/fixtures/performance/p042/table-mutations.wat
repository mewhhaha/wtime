;; Original P042 table mutation source fixture; dual MIT OR Apache-2.0.
;; Source-only: parser validation, encoding and execution are pending.
(module
  (type $zero_i32 (func (result i32)))
  (table $t 2 5 funcref)
  (table $other 2 5 funcref)
  (func $a (type $zero_i32) (result i32) i32.const 11)
  (func $b (type $zero_i32) (result i32) i32.const 22)
  (func $c (type $zero_i32) (result i32) i32.const 33)
  (elem (table $t) (i32.const 0) func $a $b)
  (elem (table $other) (i32.const 0) func $b $c)
  (elem declare func $a $b $c)
  (export "table0" (table $t))
  (export "table1" (table $other))
  (func (export "run_t") (param $index i32) (result i32)
    (call_indirect $t (type $zero_i32) (local.get $index)))
  (func (export "run_other") (param $index i32) (result i32)
    (call_indirect $other (type $zero_i32) (local.get $index)))
  (func (export "set_a") (param $index i32)
    local.get $index
    ref.func $a
    table.set $t)
  (func (export "set_b") (param $index i32)
    local.get $index
    ref.func $b
    table.set $t)
  (func (export "fill_c") (param $start i32) (param $count i32)
    local.get $start
    ref.func $c
    local.get $count
    table.fill $t)
  (func (export "grow_c") (param $delta i32) (result i32)
    ref.func $c
    local.get $delta
    table.grow $t)
  (func (export "copy_from_other") (param $dst i32) (param $src i32) (param $count i32)
    local.get $dst
    local.get $src
    local.get $count
    table.copy $t $other)
  (func (export "copy_overlap_right")
    i32.const 1
    i32.const 0
    i32.const 2
    table.copy $t $t)
  (func (export "size_t") (result i32)
    table.size $t))
