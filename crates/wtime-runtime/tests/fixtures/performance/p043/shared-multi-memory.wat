;; Original P043 shared multi-memory fixture; dual MIT OR Apache-2.0. Source-only.
(module
  (import "mem" "left" (memory $left 1 2 shared))
  (import "mem" "right" (memory $right 1 2 shared))
  (func (export "read_left") (param $address i32) (result i32)
    (i32.load8_u $left (local.get $address)))
  (func (export "read_right") (param $address i32) (result i32)
    (i32.load8_u $right (local.get $address)))
  (func (export "write_left") (param $address i32) (param $value i32)
    (i32.store8 $left (local.get $address) (local.get $value)))
  (func (export "write_right") (param $address i32) (param $value i32)
    (i32.store8 $right (local.get $address) (local.get $value)))
  (func (export "read_unaligned_left_i32") (param $address i32) (result i32)
    (i32.load $left (local.get $address)))
  ;; The harness invokes this exported helper from growth-caller's host callback.
  (func (export "grow_and_seed") (result i32)
    (drop (memory.grow $left (i32.const 1)))
    (i32.store8 $left (i32.const 65536) (i32.const 77))
    (i32.load8_u $left (i32.const 65536))))
