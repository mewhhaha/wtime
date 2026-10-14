;; CC0-1.0. Source: staged specifically for P029, 2026-10-11.
(module
  (import "host" "f" (func $f (param i32) (result i32)))
  (func (export "run") (param i32) (result i32)
    local.get 0
    call $f))
