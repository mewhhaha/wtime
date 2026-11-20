;; Straight-line code built from the lowered Decoded tier's fused pairs, so
;; that each instruction executes exactly once: 24 instructions, counting the
;; function's end. Grouping decides which pairs actually form; the unit tests
;; in execution/lowered.rs check that each pair forms, and
;; fused-expressions.wat runs them against unfused code.
(module
  (global $g (mut i32) (i32.const 5))
  (func (export "f") (param $a i32) (result i32) (local $b i32) (local $c i32)
    local.get $a      ;; local.get + local.set
    local.set $b
    local.get $b      ;; local.get + i32.const, then i32.add + local.set
    i32.const 3
    i32.add
    local.set $c
    local.get $c      ;; local.get + local.get
    local.get $b
    i32.add
    local.set $c      ;; local.set + local.get
    local.get $c
    global.set $g
    global.get $g     ;; global.get + local.set
    local.set $b
    local.get $b      ;; local.get + global.set
    global.set $g
    i32.const 7       ;; i32.const + i32.add
    global.get $g
    i32.add
    local.get $c
    i32.add
    local.set $c
    local.get $c))
