;; SPDX-License-Identifier: MIT
(module
  (memory (export "count") 1 1)
  (func (export "run") (param $arg i32) (result i32)
    i32.const 0
    i32.const 0
    i32.load
    i32.const 1
    i32.add
    i32.store
    i32.const 0
    i32.load
    i32.const 2
    i32.eq
    if (result i32)
      i32.const 12345
    else
      local.get $arg
      i32.const 1
      i32.add
    end))
