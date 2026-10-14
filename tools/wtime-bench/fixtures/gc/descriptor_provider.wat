(module
  (type $child (struct (field i32)))
  (rec
    (type $object (descriptor $descriptor) (struct (field (ref $child))))
    (type $descriptor (describes $object) (struct (field i32))))
  (func (export "make") (param $value i32) (result (ref $object))
    (struct.new_desc $object
      (struct.new $child (local.get $value))
      (struct.new $descriptor (i32.const 9))))
  (func (export "measure") (result i32)
    (struct.get $child 0
      (struct.get $object 0 (call 0 (i32.const 77))))))
