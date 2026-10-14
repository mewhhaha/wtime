;; SPDX-License-Identifier: MIT
(module
 (tag $e (export "e") (param i32))
 (func $thrower (export "thrower") (result i32)
  i32.const 13 i32.const 7 i32.add throw $e)
 (func (export "run") (result i32)
  block (result i32)
   try_table (catch $e 0)
    call $thrower drop
   end
   unreachable
  end))
