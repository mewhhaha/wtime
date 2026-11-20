;; fused-expressions.wat with a nop after every instruction, so that no
;; group of the lowered Decoded tier forms (generated from it).
(module
  (memory (export "mem") 1)
  (func $f (export "f") (param $a i32) (param $p i64) (result i64)
    (local $b i32) (local $c i32) (local $q i64)
    local.get $a
    nop
    i32.const 8
    nop
    i32.mul
    nop
    local.set $b
    nop
    local.get $b
    nop
    local.get $a
    nop
    i32.store offset=4
    nop
    local.get $b
    nop
    i32.load offset=4
    nop
    local.set $c
    nop
    local.get $p
    nop
    i64.const -3
    nop
    i64.add
    nop
    local.set $q
    nop
    local.get $b
    nop
    local.get $q
    nop
    i64.store offset=16
    nop
    local.get $c
    nop
    local.get $b
    nop
    i64.load offset=16
    nop
    local.get $q
    nop
    i64.const 7
    nop
    i64.mul
    nop
    i64.add
    nop
    local.set $q
    nop
    i64.extend_i32_u
    nop
    local.get $q
    nop
    i64.const 1
    nop
    i64.gt_u
    nop
    i64.extend_i32_u
    nop
    i64.add
    nop
    local.get $b
    nop
    i32.load8_u offset=4
    nop
    i64.extend_i32_u
    nop
    i64.add
    nop
    local.get $b
    nop
    local.get $q
    nop
    i64.store32 offset=24
    nop
    local.get $b
    nop
    i32.const 24
    nop
    i32.add
    nop
    i32.load
    nop
    i64.extend_i32_s
    nop
    i64.add
    nop)
  (func (export "deep") (param $a i32) (param $p i64) (result i64) (local $r i64)
    i32.const 1
    nop
    i32.const 2
    nop
    i32.const 3
    nop
    local.get $a
    nop
    local.get $p
    nop
    call $f
    nop
    local.set $r
    nop
    drop
    nop
    drop
    nop
    drop
    nop
    local.get $r
    nop)
  (func (export "load") (param $a i32) (result i32) (local $x i32)
    local.get $a
    nop
    i32.load offset=1
    nop
    local.set $x
    nop
    local.get $x
    nop)
  (func (export "store") (param $a i32)
    local.get $a
    nop
    local.get $a
    nop
    i32.store16 offset=1
    nop))
