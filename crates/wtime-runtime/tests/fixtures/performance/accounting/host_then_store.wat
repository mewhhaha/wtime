(module
  (import "host" "stop" (func $stop))
  (memory (export "memory") 1)
  (func (export "run") (result i32)
    ;; Source instruction PCs: 0 call $stop; 1 i32.const 0;
    ;; 2 i32.const 7; 3 i32.store; 4 i32.const 9; 5 function end.
    call $stop
    i32.const 0
    i32.const 7
    i32.store
    i32.const 9))
