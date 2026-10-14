;; SPDX-License-Identifier: CC0-1.0
(module
 (memory (export "memory") i64 1 1)
 (func (export "last_byte") (result i32)
  (i32.store8 (i64.const 65535) (i32.const 128))
  (i32.load8_s (i64.const 65535)))
 (func (export "overflow") (result i32)
  (i32.store8 (i64.const 8) (i32.const 170))
  (drop (i32.load offset=18446744073709551615 (i64.const 1)))
  (i32.store8 (i64.const 9) (i32.const 204)) (i32.const 0)))
