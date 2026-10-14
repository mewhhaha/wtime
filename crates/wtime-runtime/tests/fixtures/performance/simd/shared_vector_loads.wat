;; Original CC0 P026 fixture. Host supplies the imported shared memory contents.
(module
  (import "env" "memory" (memory 1 1 shared))
  (export "memory" (memory 0))
  (func (export "v128.load") (result v128)
    (v128.load (i32.const 0)))
  (func (export "v128.load8_lane_nonzero") (param $x v128) (result v128)
    (v128.load8_lane 8 (i32.const 5) (local.get $x))))
