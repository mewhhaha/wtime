(module
  (import "env" "mark" (func $mark))
  (func $run (export "run") (result i32)
    i32.const 73)
  (func $start
    call $mark
    unreachable)
  (start $start))
