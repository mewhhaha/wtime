(module
  (func $leaf (param i32) (result i32)
    local.get 0
    local.get 0
    i32.add)
  (func (export "run") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    call $leaf
    i32.add))
