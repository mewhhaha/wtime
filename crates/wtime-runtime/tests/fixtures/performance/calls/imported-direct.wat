;; CC0-1.0. Source: staged specifically for P029, 2026-10-11.
(module
  (import "host" "bump" (func $bump (param i32) (result i32)))
  (func $wrap (param i32) (result i32)
    local.get 0
    call $bump
    i32.const 4
    i32.add)
  (func (export "run") (param i32) (result i32)
    local.get 0
    call $wrap))
