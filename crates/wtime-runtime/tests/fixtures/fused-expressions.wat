;; Straight-line code containing every fused expression of the lowered
;; Decoded tier, some of them above operands already on the stack.
;; fused-expressions-nops.wat is the same code with a nop after every
;; instruction, which no group contains: it runs unfused.
(module
  (memory (export "mem") 1)
  (global $g (mut i32) (i32.const 5))
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
;; Fused pairs, with their effects stored: 31 instructions and the end.
  (func (export "h") (param $a i32) (local $b i32) (local $c i32)
    i32.const 0       ;; stays below until the store at offset 8
    local.get $a      ;; local.get + local.set
    local.set $b
    i32.const 4
    i32.const 1
    local.set $c      ;; local.set + local.get
    local.get $c
    i32.store
    global.get $g     ;; global.get + local.set
    local.set $c
    global.get $g
    i32.const 7       ;; i32.const + i32.add
    i32.add
    global.set $g
    local.get $b      ;; local.get + global.set
    global.set $g
    local.get $b
    local.get $c
    i32.add
    local.set $c
    global.get $g
    i32.store offset=8
    i32.const 12
    local.get $c
    i32.store
    i32.const 16
    i32.const 4
    local.get $b
    i32.add           ;; i32.add + i32.load
    i32.load
    i32.store)
  ;; Conditionals closing groups: compare + br_if (not taken, taken, and
  ;; taken out of the function) and compare + if (true, false with no else,
  ;; whose end is skipped, and false with an else).
  (func $g (export "g") (param $a i32) (param $p i64) (result i64)
    (local $x i32) (local $y i64)
    local.get $p      ;; local.get + i32.wrap_i64 + i32.const + i32.mul
    i32.wrap_i64
    i32.const 3
    i32.mul
    local.set $x
    local.get $a      ;; local.get + i64.extend_i32_u
    i64.extend_i32_u
    local.set $y
    block $out
      local.get $x    ;; not taken
      local.get $a
      i32.lt_u
      br_if $out
      local.get $y
      i64.const 10
      i64.add
      local.set $y
      local.get $a    ;; i32.eqz + br_if, not taken
      i32.eqz
      br_if $out
      i32.const 5     ;; i32.ne + br_if, not taken
      local.get $a
      i32.ne
      br_if $out
      i32.const 5     ;; i32.eq + br_if, taken
      local.get $a
      i32.eq
      br_if $out
      i64.const 1000
      local.set $y
    end
    i32.const 1       ;; i32.lt_u + if, true
    local.get $a
    i32.lt_u
    if
      local.get $y
      i64.const 2
      i64.mul
      local.set $y
    end
    local.get $x      ;; false, no else
    local.get $a
    i32.lt_s
    if
      i64.const 7777
      local.set $y
    end
    local.get $a      ;; i32.eqz + if, false, no else
    i32.eqz
    if
      i64.const 8888
      local.set $y
    end
    local.get $a      ;; false, with an else
    i32.const 5
    i32.ne
    if (result i64)
      i64.const 1
    else
      local.get $y
    end
    local.get $x      ;; false, no else, with a param: its end is skipped
    i32.const 0
    if (param i32) (result i32)
      i32.const 1
      i32.add
    end
    drop
    i32.const 5       ;; i32.eq + br_if out of the function
    local.get $a
    i32.eq
    br_if 0
    drop
    i64.const -1)
  ;; Runs g above three operands of its own.
  (func (export "deepg") (param $a i32) (param $p i64) (result i64) (local $r i64)
    i32.const 1
    i32.const 2
    i32.const 3
    local.get $a
    local.get $p
    call $g
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
