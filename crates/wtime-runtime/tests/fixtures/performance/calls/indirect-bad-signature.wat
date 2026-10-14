;; CC0-1.0. Source: staged specifically for P029, 2026-10-11.
;; Module validates: slot 0 contains a different signature, so call_indirect traps.
(module
  (type $expected (func (param i32) (result i32)))
  (type $actual (func (param i64) (result i64)))
  (func $wrong (type $actual) (param i64) (result i64)
    local.get 0)
  (table 1 funcref)
  (elem (i32.const 0) $wrong)
  (func (export "run") (result i32)
    i32.const 3
    i32.const 0
    call_indirect (type $expected)))
