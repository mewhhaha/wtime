(module
  (func (export "run") (result i32 i64)
    ;; Source instruction PCs: 0 block (result i32 i64); 1 i32.const 7;
    ;; 2 i64.const 9; 3 br 0 (carries [i32 7, i64 9], in that order);
    ;; 4 unreachable; 5 block end; 6 function end.
    (block (result i32 i64)
      i32.const 7
      i64.const 9
      br 0
      unreachable)))
