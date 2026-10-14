;; SPDX-License-Identifier: CC0-1.0
(module
 (tag $failure (export "failure") (param i32))
 (memory (export "memory") 1)
 (func $raise (param i32)
  local.get 0 throw $failure)
 (func (export "caught") (param i32) (result i32) (local i32)
  local.get 0 i32.const 1 i32.add local.set 1
  try_table (catch $failure 0)
   i32.const 0 local.get 1 i32.store
   local.get 1 call $raise
  end
  i32.const -1)
 (func (export "uncaught") (param i32)
  local.get 0 i32.const 2 i32.add throw $failure)
 (func (export "nested_uncaught") (param i32)
  local.get 0 i32.const 3 i32.add call $raise))
