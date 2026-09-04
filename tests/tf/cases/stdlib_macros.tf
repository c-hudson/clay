;; preload: stdlib.tf
;; stdlib_macros.tf - exercises several stdlib.tf one-liners (finding C.1's
;; -i flag and C.11's missing natives mean most of these fail in Clay even
;; with the library preloaded - see "Missing TF commands" in finding B).
;; isvar("HOME") checks a variable that's always set (the user's home
;; directory); isvar("no_such_var_xyz") checks one that never is.
/eval /echo $(/first a b c)|$(/rest a b c)|$(/last a b c)|$(/nth 2 a b c)|$(/escape " a"b")|$(/replace a o banana)
/eval /echo isvar_home=$[isvar("HOME")] isvar_missing=$[isvar("no_such_var_xyz")]
/set flag=0
/toggle flag
/eval /echo flag=%flag
/not /test 1
/eval /echo not1=%?
/not /test 0
/eval /echo not0=%?
/expr 1+2
/eval /echo expr_result=%?
/quit
