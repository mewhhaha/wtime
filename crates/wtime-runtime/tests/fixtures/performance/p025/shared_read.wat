(module
  (import "env" "memory" (memory 1 1 shared))
  (func (export "run") (result i32)
    (i32.load offset=1 (i32.const 0))))
