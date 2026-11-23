;; SPDX-License-Identifier: CC0-1.0
(module
 (type $result (func (result i32)))
 (table (export "table") 2 funcref)
 (func $a (export "a") (type $result) (i32.const 17))
 (func $b (export "b") (type $result) (i32.const 29))
 (elem (i32.const 0) $a)
 (func (export "run") (result i32) (call_indirect (type $result) (i32.const 0))))
