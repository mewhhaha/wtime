(module
  (func (export "step") (result i32)
    ;; Module-local body 0, source PCs: 0 i32.const 7; 1 i32.const 2;
    ;; 2 i32.add; 3 function end.
    i32.const 7
    i32.const 2
    i32.add))
