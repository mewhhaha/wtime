(module
  (memory $mem (export "memory") 1 2 shared)
  (data (i32.const 4) "\25\00\00\00")
  (func (export "run") (param $base i32) (result i32)
    local.get $base
    i32.const 4
    i32.add
    i32.load $mem))
