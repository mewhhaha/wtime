(module
  (import "host" "tick" (func $tick))
  (func (export "run") (result i32)
    ;; Source instruction PCs: 0 call $tick; 1 i32.const 9; 2 function end.
    call $tick
    i32.const 9))
