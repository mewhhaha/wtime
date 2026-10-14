;; SPDX-License-Identifier: Apache-2.0 OR MIT
(module (func (export "run") (result v128)
 v128.const i8x16 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15
 v128.const i8x16 101 102 103 104 105 106 107 108 109 110 111 112 113 114 115 116
 i8x16.shuffle 31 0 17 14 22 5 16 15 8 25 3 30 19 12 27 6))
