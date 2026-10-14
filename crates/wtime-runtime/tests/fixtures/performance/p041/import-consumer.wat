(module
  (import "env" "f" (func $f (param i32) (result i32)))
  (func $main (param i32) (result i32)
    local.get 0
    call $f)
  (export "main" (func $main)))
