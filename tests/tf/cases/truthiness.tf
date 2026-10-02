;; truthiness.tf - how TF turns a string into a number or a boolean: a
;; non-numeric string is 0 (false), a string with a leading number is that
;; number (" 7", "0.5", "-3x", "12abc"), "&" and "|" return an operand rather
;; than 0/1, and "==" compares numerically.
/eval /echo a=$["abc" ? 1 : 0] b=$["12abc" ? 1 : 0] c=$["on" ? 1 : 0] d=$["off" ? 1 : 0] e=$["" ? 1 : 0] f=$["0" ? 1 : 0] g=$[" 7" ? 1 : 0] h=$["0.5" ? 1 : 0] i=$["-3x" ? 1 : 0]
/set foo=off
/eval /echo foonot=$[!foo] fooq=$[foo ? 1 : 0]
/set bar=abc
/eval /echo barnot=$[!bar] barq=$[bar ? 1 : 0] barneg=$[-bar]
/eval /echo cmp1=$["abc" == 0] cmp2=$["10" == 10] cmp3=$["10x" == 10] cmp4=$["abc" == "abc"]
/eval /echo andstr=$["abc" & "def"] orstr=$["abc" | "def"] or0=$[0 | ""] and3=$[1 & 2 & 3]
/eval /echo not1=$[!"1"] notabc=$[!"abc"] notempty=$[!""]
/eval /echo plus=$["12abc" + 1] plus2=$["abc" + 1] plus3=$["1.5x" + 1] mul=$[" 3" * 2]
/set y=hello
/eval /if (y) /echo y-true%; /else /echo y-false%; /endif
/set z=5
/eval /if (z) /echo z-true%; /else /echo z-false%; /endif
/quit
