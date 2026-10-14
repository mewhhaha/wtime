;; SPDX-License-Identifier: MIT
;; P058 branch result-move fixture.
;; Source PC 4 is br 0; its target is function end at PC 6.
(module
  (func (export "run") (result i32 i64)
    (block (result i32 i64)
      i32.const 99
      i32.const 7
      i64.const 9
      br 0)))
