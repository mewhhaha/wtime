;; SPDX-License-Identifier: MIT
;; P058 numeric/local stack-slot fixture.
;; Source PC 0..15 are sequential instructions; PC 15 is the implicit function end.
(module
  (func (export "run") (param $left i32) (param $right i32) (result i32)
    (local $scratch i32)
    local.get $left
    local.tee $scratch
    i32.const 3
    i32.sub
    local.set $scratch
    i64.const 40
    i64.const 2
    i64.sub
    drop
    f32.const -0x0p+0
    i32.reinterpret_f32
    drop
    local.get $scratch
    local.get $right
    i32.sub))
