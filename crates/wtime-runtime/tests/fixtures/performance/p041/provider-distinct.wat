(module
  (func $plus10 (param i32) (result i32)
    local.get 0
    i32.const 10
    i32.add)
  (func $plus20 (param i32) (result i32)
    local.get 0
    i32.const 20
    i32.add)
  (export "f10" (func $plus10))
  (export "f20" (func $plus20)))
