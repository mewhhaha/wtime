(module
  (memory (export "memory") 1 1)
  (data (i32.const 24) "\01\02\03\04")
  (func $load (param $base i32) (result i32)
    local.get $base
    i32.const 24
    i32.add
    i32.load)
  (func (export "run") (param $base i32) (result i32)
    i32.const 91
    local.get $base
    call $load
    i32.add))
