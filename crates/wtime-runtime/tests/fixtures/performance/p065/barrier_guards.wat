;; SPDX-License-Identifier: MIT
(module
 (type $box (struct (field i32)))
 (func (export "ref_local") (result i32) (local externref)
  i32.const 1 drop i32.const 42)
 (func (export "vector_local") (result i32) (local v128)
  i32.const 1 drop i32.const 42)
 (func (export "numeric_gc") (result i32)
  i32.const 13 struct.new $box drop i32.const 42)
 (func (export "numeric_ref") (result i32)
  i32.const 1 drop ref.null extern drop i32.const 42))
