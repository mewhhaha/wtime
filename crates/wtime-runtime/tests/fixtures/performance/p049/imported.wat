(module
  (import "env" "memory" (memory $mem 1 2))
  (func (export "run") (param $base i32) (result i32)
    local.get $base
    i32.const 4
    i32.add
    i32.load $mem))
