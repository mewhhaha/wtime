(module
  (type $object (struct (field i32)))
  (type $run (func (result i32)))
  (type $step (func (param i32) (result i32)))
  (type $cont (cont $step))
  (tag $yield (type $step))
  (func (export "run") (type $run) (local $saved (ref null $object))
    (local.set $saved (struct.new $object (i32.const 77)))
    i32.const 5
    suspend $yield
    local.get $saved
    struct.get $object 0
    i32.add)
  (func (export "resume") (param (ref $cont)) (result i32)
    i32.const 9
    local.get 0
    resume $cont))
