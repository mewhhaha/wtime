(module
  (import "env" "x" (memory 1 1 shared))
  (import "env" "y" (memory 1 1 shared))
  (func (export "store_x_load_y") (result i32)
    (i32.atomic.store 0 (i32.const 0) (i32.const 1))
    (i32.atomic.load 1 (i32.const 0)))
  (func (export "store_y_load_x") (result i32)
    (i32.atomic.store 1 (i32.const 0) (i32.const 1))
    (i32.atomic.load 0 (i32.const 0))))
