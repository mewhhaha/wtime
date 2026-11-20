;; fused-expressions.wat with a nop after every instruction, so that no
;; group of the lowered Decoded tier forms (generated from it).
(module
  (memory (export "mem") 1)
  (global $g (mut i32) (i32.const 5))
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
  (func (export "h") (param $a i32) (local $b i32) (local $c i32)
    i32.const 0
    nop
    local.get $a
    nop
    local.set $b
    nop
    i32.const 4
    nop
    i32.const 1
    nop
    local.set $c
    nop
    local.get $c
    nop
    i32.store
    nop
    global.get $g
    nop
    local.set $c
    nop
    global.get $g
    nop
    i32.const 7
    nop
    i32.add
    nop
    global.set $g
    nop
    local.get $b
    nop
    global.set $g
    nop
    local.get $b
    nop
    local.get $c
    nop
    i32.add
    nop
    local.set $c
    nop
    global.get $g
    nop
    i32.store offset=8
    nop
    i32.const 12
    nop
    local.get $c
    nop
    i32.store
    nop
    i32.const 16
    nop
    i32.const 4
    nop
    local.get $b
    nop
    i32.add
    nop
    i32.load
    nop
    i32.store
    nop)
  (func $g (export "g") (param $a i32) (param $p i64) (result i64)
    (local $x i32) (local $y i64)
    local.get $p
    nop
    i32.wrap_i64
    nop
    i32.const 3
    nop
    i32.mul
    nop
    local.set $x
    nop
    local.get $a
    nop
    i64.extend_i32_u
    nop
    local.set $y
    nop
    block $out
    nop
      local.get $x
      nop
      local.get $a
      nop
      i32.lt_u
      nop
      br_if $out
      nop
      local.get $y
      nop
      i64.const 10
      nop
      i64.add
      nop
      local.set $y
      nop
      local.get $a
      nop
      i32.eqz
      nop
      br_if $out
      nop
      i32.const 5
      nop
      local.get $a
      nop
      i32.ne
      nop
      br_if $out
      nop
      i32.const 5
      nop
      local.get $a
      nop
      i32.eq
      nop
      br_if $out
      nop
      i64.const 1000
      nop
      local.set $y
      nop
    end
    nop
    i32.const 1
    nop
    local.get $a
    nop
    i32.lt_u
    nop
    if
    nop
      local.get $y
      nop
      i64.const 2
      nop
      i64.mul
      nop
      local.set $y
      nop
    end
    nop
    local.get $x
    nop
    local.get $a
    nop
    i32.lt_s
    nop
    if
    nop
      i64.const 7777
      nop
      local.set $y
      nop
    end
    nop
    local.get $a
    nop
    i32.eqz
    nop
    if
    nop
      i64.const 8888
      nop
      local.set $y
      nop
    end
    nop
    local.get $a
    nop
    i32.const 5
    nop
    i32.ne
    nop
    if (result i64)
    nop
      i64.const 1
      nop
    else
    nop
      local.get $y
      nop
    end
    nop
    local.get $x
    nop
    i32.const 0
    nop
    if (param i32) (result i32)
    nop
      i32.const 1
      nop
      i32.add
      nop
    end
    nop
    drop
    nop
    i32.const 5
    nop
    local.get $a
    nop
    i32.eq
    nop
    br_if 0
    nop
    drop
    nop
    i64.const -1
    nop)
  (func (export "deepg") (param $a i32) (param $p i64) (result i64) (local $r i64)
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
    call $g
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
