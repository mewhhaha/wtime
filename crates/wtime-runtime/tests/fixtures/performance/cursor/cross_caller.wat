(module
  (import "other" "step" (func $step (result i32)))
  ;; Imported function is index 0; this run body is module-local body 0,
  ;; absolute function index 1. Source PCs: 0 call $step; 1 i32.const 3;
  ;; 2 i32.add; 3 function end.
  (func (export "run") (result i32)
    call $step
    i32.const 3
    i32.add))
