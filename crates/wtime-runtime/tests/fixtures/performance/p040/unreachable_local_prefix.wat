;; Copyright (c) 2026. SPDX-License-Identifier: MIT
(module
  (func (export "run") (result i32) (local $visible i32)
    i32.const 9                 ;; pc 0
    local.set $visible          ;; pc 1; state is now visible=9
    unreachable                 ;; pc 2; traps after its source event is charged
    local.get $visible          ;; pc 3; validation-polymorphic, never executes
  )                             ;; pc 4; implicit function end
)
