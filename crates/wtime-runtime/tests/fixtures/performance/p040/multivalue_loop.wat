;; Copyright (c) 2026. SPDX-License-Identifier: MIT
(module
  (func (export "run") (result i32 i64) (local $i i32)
    i32.const 2                 ;; pc 0
    i64.const 40                ;; pc 1
    block (param i32 i64) (result i32 i64) ;; pc 2
      loop (param i32 i64) (result i32 i64) ;; pc 3
        local.get $i            ;; pc 4
        i32.const 2             ;; pc 5
        i32.ge_u                ;; pc 6
        br_if 1                 ;; pc 7; third visit exits with carried pair
        local.get $i            ;; pc 8
        i32.const 1             ;; pc 9
        i32.add                 ;; pc 10
        local.set $i            ;; pc 11
        br 0                    ;; pc 12
      end                       ;; pc 13; skipped by loop backedge/exit branch
    end                         ;; pc 14
  )                             ;; pc 15; implicit function end
)
