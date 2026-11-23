(module
  (type $run (func (result i32)))
  (type $step (func (param i32) (result i32)))
  (type $cont (cont $step))
  (tag $yield (type $step))
  (func $leaf (type $run) (local $saved i32)
    i32.const 1000
    local.set $saved
    i32.const 7
    suspend $yield
    local.get $saved
    i32.add)
  (func $frame_0 (type $run) (local $saved_0 i32)
    i32.const 1
    local.set $saved_0
    call $frame_1
    local.get $saved_0
    i32.add)
  (func $frame_1 (type $run) (local $saved_1 i32)
    i32.const 2
    local.set $saved_1
    call $frame_2
    local.get $saved_1
    i32.add)
  (func $frame_2 (type $run) (local $saved_2 i32)
    i32.const 3
    local.set $saved_2
    call $frame_3
    local.get $saved_2
    i32.add)
  (func $frame_3 (type $run) (local $saved_3 i32)
    i32.const 4
    local.set $saved_3
    call $frame_4
    local.get $saved_3
    i32.add)
  (func $frame_4 (type $run) (local $saved_4 i32)
    i32.const 5
    local.set $saved_4
    call $frame_5
    local.get $saved_4
    i32.add)
  (func $frame_5 (type $run) (local $saved_5 i32)
    i32.const 6
    local.set $saved_5
    call $frame_6
    local.get $saved_5
    i32.add)
  (func $frame_6 (type $run) (local $saved_6 i32)
    i32.const 7
    local.set $saved_6
    call $frame_7
    local.get $saved_6
    i32.add)
  (func $frame_7 (type $run) (local $saved_7 i32)
    i32.const 8
    local.set $saved_7
    call $frame_8
    local.get $saved_7
    i32.add)
  (func $frame_8 (type $run) (local $saved_8 i32)
    i32.const 9
    local.set $saved_8
    call $frame_9
    local.get $saved_8
    i32.add)
  (func $frame_9 (type $run) (local $saved_9 i32)
    i32.const 10
    local.set $saved_9
    call $frame_10
    local.get $saved_9
    i32.add)
  (func $frame_10 (type $run) (local $saved_10 i32)
    i32.const 11
    local.set $saved_10
    call $frame_11
    local.get $saved_10
    i32.add)
  (func $frame_11 (type $run) (local $saved_11 i32)
    i32.const 12
    local.set $saved_11
    call $frame_12
    local.get $saved_11
    i32.add)
  (func $frame_12 (type $run) (local $saved_12 i32)
    i32.const 13
    local.set $saved_12
    call $frame_13
    local.get $saved_12
    i32.add)
  (func $frame_13 (type $run) (local $saved_13 i32)
    i32.const 14
    local.set $saved_13
    call $frame_14
    local.get $saved_13
    i32.add)
  (func $frame_14 (type $run) (local $saved_14 i32)
    i32.const 15
    local.set $saved_14
    call $frame_15
    local.get $saved_14
    i32.add)
  (func $frame_15 (type $run) (local $saved_15 i32)
    i32.const 16
    local.set $saved_15
    call $frame_16
    local.get $saved_15
    i32.add)
  (func $frame_16 (type $run) (local $saved_16 i32)
    i32.const 17
    local.set $saved_16
    call $frame_17
    local.get $saved_16
    i32.add)
  (func $frame_17 (type $run) (local $saved_17 i32)
    i32.const 18
    local.set $saved_17
    call $frame_18
    local.get $saved_17
    i32.add)
  (func $frame_18 (type $run) (local $saved_18 i32)
    i32.const 19
    local.set $saved_18
    call $frame_19
    local.get $saved_18
    i32.add)
  (func $frame_19 (type $run) (local $saved_19 i32)
    i32.const 20
    local.set $saved_19
    call $frame_20
    local.get $saved_19
    i32.add)
  (func $frame_20 (type $run) (local $saved_20 i32)
    i32.const 21
    local.set $saved_20
    call $frame_21
    local.get $saved_20
    i32.add)
  (func $frame_21 (type $run) (local $saved_21 i32)
    i32.const 22
    local.set $saved_21
    call $frame_22
    local.get $saved_21
    i32.add)
  (func $frame_22 (type $run) (local $saved_22 i32)
    i32.const 23
    local.set $saved_22
    call $frame_23
    local.get $saved_22
    i32.add)
  (func $frame_23 (type $run) (local $saved_23 i32)
    i32.const 24
    local.set $saved_23
    call $frame_24
    local.get $saved_23
    i32.add)
  (func $frame_24 (type $run) (local $saved_24 i32)
    i32.const 25
    local.set $saved_24
    call $frame_25
    local.get $saved_24
    i32.add)
  (func $frame_25 (type $run) (local $saved_25 i32)
    i32.const 26
    local.set $saved_25
    call $frame_26
    local.get $saved_25
    i32.add)
  (func $frame_26 (type $run) (local $saved_26 i32)
    i32.const 27
    local.set $saved_26
    call $frame_27
    local.get $saved_26
    i32.add)
  (func $frame_27 (type $run) (local $saved_27 i32)
    i32.const 28
    local.set $saved_27
    call $frame_28
    local.get $saved_27
    i32.add)
  (func $frame_28 (type $run) (local $saved_28 i32)
    i32.const 29
    local.set $saved_28
    call $frame_29
    local.get $saved_28
    i32.add)
  (func $frame_29 (type $run) (local $saved_29 i32)
    i32.const 30
    local.set $saved_29
    call $frame_30
    local.get $saved_29
    i32.add)
  (func $frame_30 (type $run) (local $saved_30 i32)
    i32.const 31
    local.set $saved_30
    call $frame_31
    local.get $saved_30
    i32.add)
  (func $frame_31 (type $run) (local $saved_31 i32)
    i32.const 32
    local.set $saved_31
    call $leaf
    local.get $saved_31
    i32.add)
  (func (export "run") (type $run) call $frame_0)
  (func (export "resume") (param (ref $cont)) (result i32)
    i32.const 9
    local.get 0
    resume $cont))
