;; SPDX-License-Identifier: MIT
;; P060 nested multi-value br_table fixture. Original fixture authored for this repository.
(module
  (func (export "run") (param $which i32) (result i32 i64)
    (block (result i32 i64)
      (block (result i32 i64)
        i32.const 99
        i32.const 7
        i64.const 9
        local.get $which
        br_table 0 1 2
      )
    )
  )
)
