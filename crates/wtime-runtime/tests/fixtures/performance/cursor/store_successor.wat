(module
  (memory (export "memory") 1)
  (func (export "run") (result i32)
    ;; Source instruction PCs: 0 i32.const 0; 1 i32.const 7; 2 i32.store;
    ;; 3 i32.const 0; 4 i32.load; 5 function end.
    i32.const 0
    i32.const 7
    i32.store
    i32.const 0
    i32.load))
