(module
  (type $child (struct (field i32)))
  (rec
    (type $object (descriptor $descriptor) (struct (field (ref $child))))
    (type $descriptor (describes $object) (struct (field i32))))
  (import "provider" "make" (func $make (param i32) (result (ref $object))))
  (func (export "run") (param $value i32) (result i32)
    (struct.get $child 0 (struct.get $object 0 (call $make (local.get $value))))))
