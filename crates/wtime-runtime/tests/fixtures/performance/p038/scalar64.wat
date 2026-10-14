;; SPDX-License-Identifier: Apache-2.0 OR MIT
(module
  (func (export "run") (result i64)
    i64.const 9223372036854775807
    i64.const 1
    i64.add
    i64.const -1
    i64.mul))
