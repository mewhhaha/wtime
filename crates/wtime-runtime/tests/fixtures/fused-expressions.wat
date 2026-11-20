;; Straight-line code containing every fused expression of the lowered
;; Decoded tier, some of them above operands already on the stack.
;; fused-expressions-nops.wat is the same code with a nop after every
;; instruction, which no group contains: it runs unfused.
(module
  (memory (export "mem") 1)
  (func $f (export "f") (param $a i32) (param $p i64) (result i64)
    (local $b i32) (local $c i32) (local $q i64)
    local.get $a      ;; local.get + i32.const + i32.mul + local.set
    i32.const 8
    i32.mul
    local.set $b
    local.get $b      ;; local.get + local.get + i32.store
    local.get $a
    i32.store offset=4
    local.get $b      ;; local.get + i32.load + local.set
    i32.load offset=4
    local.set $c
    local.get $p      ;; local.get + i64.const + i64.add + local.set
    i64.const -3
    i64.add
    local.set $q
    local.get $b      ;; local.get + local.get + i64.store
    local.get $q
    i64.store offset=16
    local.get $c      ;; stays below the groups that follow
    local.get $b      ;; local.get + i64.load
    i64.load offset=16
    local.get $q      ;; local.get + i64.const + i64.mul
    i64.const 7
    i64.mul
    i64.add
    local.set $q
    i64.extend_i32_u
    local.get $q      ;; local.get + i64.const + i64.gt_u
    i64.const 1
    i64.gt_u
    i64.extend_i32_u
    i64.add
    local.get $b      ;; local.get + i32.load8_u
    i32.load8_u offset=4
    i64.extend_i32_u
    i64.add
    local.get $b      ;; local.get + local.get + i64.store32
    local.get $q
    i64.store32 offset=24
    local.get $b      ;; local.get + i32.const + i32.add (no set)
    i32.const 24
    i32.add
    i32.load
    i64.extend_i32_s
    i64.add)
  ;; Runs f above three operands of its own.
  (func (export "deep") (param $a i32) (param $p i64) (result i64) (local $r i64)
    i32.const 1
    i32.const 2
    i32.const 3
    local.get $a
    local.get $p
    call $f
    local.set $r
    drop
    drop
    drop
    local.get $r)
  ;; Out-of-bounds accesses through fused groups trap as unfused ones do.
  (func (export "load") (param $a i32) (result i32) (local $x i32)
    local.get $a
    i32.load offset=1
    local.set $x
    local.get $x)
  (func (export "store") (param $a i32)
    local.get $a
    local.get $a
    i32.store16 offset=1))
