(module
  (import "env" "collect" (func $collect (result i32)))
  (type $child (struct (field i32)))
  (rec
    (type $object (descriptor $descriptor) (struct (field (ref $child))))
    (type $descriptor (describes $object) (struct (field i32))))
  (func (export "run") (param $value i32) (result i32)
    (struct.get $child 0
      (struct.get $object 0
        (struct.new_desc $object
          (struct.new $child (local.get $value))
          (struct.new $descriptor (call $collect)))))))
