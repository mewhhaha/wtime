;; Original CC0 P076 fixture. Both imports may alias one shared backing.
(module
  (import "env" "a" (memory $a 1 2 shared))
  (import "env" "b" (memory $b 1 2 shared))
  (global $state (export "state") (mut i32) (i32.const 0))
  (func (export "scalar") (result i32 i32 i32 i32 i64 i64 f32 f64)
    (i32.load8_s $a (i32.const 1))
    (i32.load8_u $a (i32.const 1))
    (i32.load16_s $b (i32.const 3))
    (i32.load16_u $b (i32.const 3))
    (i64.load32_s $a (i32.const 5))
    (i64.load $b (i32.const 9))
    (f32.load $a (i32.const 17))
    (f64.load $b (i32.const 21)))
  (func (export "vector") (result v128)
    (v128.load $b (i32.const 1)))
  (func (export "lane") (param v128) (result v128)
    (v128.load16_lane $a 5 (i32.const 3) (local.get 0)))
  (func (export "prefix") (result i32)
    i32.const 40
    i32.const 170
    i32.store8 $a
    i32.const 1
    i32.load8_u $b
    global.set $state
    i32.const 65535
    i32.load $a
    drop
    global.get $state)
  (func (export "grow") (result i32)
    (memory.grow $a (i32.const 1))))
