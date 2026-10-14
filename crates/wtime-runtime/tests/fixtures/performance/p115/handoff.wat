;; SPDX-License-Identifier: CC0-1.0
(module
 (type $box (struct (field i32)))
 (memory (export "memory") 1)
 (func (export "store_after_prefix") (param i32) (result i32) (local i32)
  local.get 0 i32.const 3 i32.add local.set 1
  i32.const 0 local.get 1 i32.store
  i32.const 0 memory.grow drop
  local.get 0 i32.const 9 i32.add)
 (func (export "branch_over_complex") (param i32) (result i32)
  local.get 0 i32.const 2 i32.mul
  block $done
   local.get 0 i32.eqz br_if $done
   i32.const 91 struct.new $box drop
  end
  i32.const 5 i32.add))
