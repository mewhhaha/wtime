;; Copyright (c) 2026. SPDX-License-Identifier: MIT
(module
  (func (export "branch") (param $take i32) (result i32)
    block (result i32)          ;; pc 0
      i32.const 42              ;; pc 1
      local.get $take           ;; pc 2
      br_if 1                   ;; pc 3; taken target is function label
      drop                      ;; pc 4; only fallthrough
      i32.const 7               ;; pc 5
    end                         ;; pc 6
  )                             ;; pc 7; implicit function end
  (func (export "return") (param $take i32) (result i32)
    local.get $take             ;; pc 0
    if (result i32)             ;; pc 1
      i32.const 42              ;; pc 2
      return                    ;; pc 3
    else                        ;; pc 4
      i32.const 7               ;; pc 5
    end                         ;; pc 6
  )                             ;; pc 7; implicit function end
)
