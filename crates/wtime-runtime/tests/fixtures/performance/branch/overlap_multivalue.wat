;; SPDX-License-Identifier: CC0-1.0
(module
  (func (export "pair") (result i32 i64)
    ;; The result tail [7, 9] overlaps its destination at stack height zero;
    ;; 99 is the value that keep must discard.
    (block (result i32 i64)
      i32.const 99
      i32.const 7
      i64.const 9
      br 0))
  (func (export "zero") (result i32)
    ;; Zero-value keep must discard the temporary and return 11.
    (block
      i32.const 123
      br 0)
    i32.const 11))
