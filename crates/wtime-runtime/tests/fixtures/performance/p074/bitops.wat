(module
  (global $state (export "state") (mut v128)
    (v128.const i8x16 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15))

  (func (export "run") (result i32)
    global.get $state
    v128.not
    global.set $state

    global.get $state
    v128.const i8x16 16 33 50 67 84 101 118 135 152 169 186 203 220 237 254 15
    v128.and
    global.set $state

    global.get $state
    v128.const i8x16 16 33 50 67 84 101 118 135 152 169 186 203 220 237 254 15
    v128.andnot
    global.set $state

    global.get $state
    v128.const i8x16 16 33 50 67 84 101 118 135 152 169 186 203 220 237 254 15
    v128.or
    global.set $state

    global.get $state
    v128.const i8x16 16 33 50 67 84 101 118 135 152 169 186 203 220 237 254 15
    v128.xor
    global.set $state

    global.get $state
    v128.const i8x16 16 33 50 67 84 101 118 135 152 169 186 203 220 237 254 15
    v128.const i8x16 0 255 128 1 85 170 127 254 0 255 128 1 85 170 127 254
    v128.bitselect
    global.set $state

    global.get $state
    v128.any_true)

  (func (export "fallback") (result v128)
    v128.const i8x16 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15
    v128.const i8x16 16 33 50 67 84 101 118 135 152 169 186 203 220 237 254 15
    i8x16.add)
)
