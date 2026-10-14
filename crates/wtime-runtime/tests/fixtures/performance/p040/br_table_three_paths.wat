;; Copyright (c) 2026. SPDX-License-Identifier: MIT
(module
  (func (export "run") (param $which i32) (result i32)
    block (result i32)          ;; pc 0; outer label, depth 1
      block (result i32)        ;; pc 1; inner label, depth 0
        i32.const 7             ;; pc 2; common branch payload
        local.get $which        ;; pc 3
        br_table 0 1 2          ;; pc 4; explicit 0, explicit 1, default depth 2
      end                       ;; pc 5; depth 0 resumes here
      i32.const 1000            ;; pc 6
      i32.add                   ;; pc 7
    end                         ;; pc 8; depth 1 resumes here
    i32.const 100               ;; pc 9
    i32.add                     ;; pc 10
  )                             ;; pc 11; default depth 2 returns directly
)
