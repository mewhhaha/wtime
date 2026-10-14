(module
  (memory (export "memory") 1)
  (global $state (export "state") (mut i32) (i32.const -19))
  (func (export "run") (param i32) (result i32) (local i32)
    i32.const 0
    i32.const 42
    i32.store
    local.get 0
    i32.const 7
    i32.add
    local.set 1
    local.get 1
    global.set 0
    i32.const 4
    local.get 1
    i32.store
    local.get 1))
