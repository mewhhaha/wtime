;; Copyright (c) 2026. SPDX-License-Identifier: MIT
(module
  (func (export "run") (param $outer i32) (param $inner i32) (result i32 i64)
    local.get $outer            ;; pc 0
    if (result i32 i64)         ;; pc 1
      local.get $inner          ;; pc 2
      if (result i32 i64)       ;; pc 3
        i32.const 41            ;; pc 4
        i64.const 420           ;; pc 5
      else                      ;; pc 6
        i32.const 42            ;; pc 7
        i64.const 421           ;; pc 8
      end                       ;; pc 9
    else                        ;; pc 10
      i32.const -1              ;; pc 11
      i64.const -2              ;; pc 12
    end                         ;; pc 13
  )                             ;; pc 14; implicit function end
)
