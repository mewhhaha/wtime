(module
  (type $object (struct (field i32)))
  (type $result (func (result (ref $object))))
  (type $exact_result (func (result (ref (exact $object)))))
  (type $resume_func (func (param (ref $object)) (result (ref $object))))
  (type $resume_cont (cont $resume_func))
  (import "provider" "yield" (tag $yield (result (ref $object))))
  (func (export "run") (type $result) (suspend $yield))
  (func (export "resume") (param (ref $resume_cont)) (result i32)
    (struct.get $object 0
      (resume $resume_cont (struct.new $object (i32.const 77)) (local.get 0)))))
