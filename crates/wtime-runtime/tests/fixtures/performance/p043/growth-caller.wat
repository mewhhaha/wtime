;; Original P043 growth caller fixture; dual MIT OR Apache-2.0. Source-only.
(module
  (import "mem" "left" (memory $left 1 2 shared))
  (import "env" "grow_and_seed" (func $grow_and_seed))
  (func (export "read_before_reentry") (result i32)
    (i32.load8_u $left (i32.const 0)))
  (func (export "read_after_host_growth") (result i32)
    (call $grow_and_seed)
    (i32.load8_u $left (i32.const 65536))))
