(module
  (import "env" "host" (func $host (param i32) (result i32)))
  (func $ordinary (param $n i32) (result i32)
    local.get $n
    call $host)
  (func $tail (param $n i32) (result i32)
    local.get $n
    return_call $host)
  (func $recurse (param $n i32) (result i32)
    local.get $n
    i32.eqz
    if (result i32)
      i32.const 0
    else
      local.get $n
      i32.const 1
      i32.sub
      call $recurse
    end)
  (func $tail_recurse (param $n i32) (result i32)
    local.get $n
    i32.eqz
    if (result i32)
      i32.const 0
    else
      local.get $n
      i32.const 1
      i32.sub
      return_call $tail_recurse
    end)
  (export "ordinary" (func $ordinary))
  (export "tail" (func $tail))
  (export "recurse" (func $recurse))
  (export "tail_recurse" (func $tail_recurse)))
