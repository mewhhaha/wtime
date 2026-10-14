(module
  (func (export "run") (param i32) (result i32) (local i32)
    local.get 0
    i32.const 2147483647
    i32.add
    local.set 1
    local.get 1))
