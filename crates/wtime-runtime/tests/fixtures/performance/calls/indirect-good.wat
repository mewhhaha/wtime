;; CC0-1.0. Source: staged specifically for P029, 2026-10-11.
(module
  (type $unary (func (param i32) (result i32)))
  (func $target (type $unary) (param i32) (result i32)
    local.get 0
    i32.const 7
    i32.add)
  (table 1 funcref)
  (elem (i32.const 0) $target)
  (func (export "run") (param i32) (result i32)
    local.get 0
    i32.const 0
    call_indirect (type $unary)))
