;; Original minimal example; binary generated with wtime-conformance encode.
(module
  (type $run (func (result i32)))
  (type $step (func (param i32) (result i32)))
  (type $cont (cont $step))
  (tag $yield (type $step))
  (func (export "run") (type $run)
    i32.const 7
    suspend $yield)
  (func (export "resume") (param (ref $cont)) (result i32)
    i32.const 9
    local.get 0
    resume $cont))
