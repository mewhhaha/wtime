(module
  (import "env" "ref" (func $ref (result externref)))
  (func $run (export "run") (result i32)
    call $ref
    drop
    i32.const 7))
