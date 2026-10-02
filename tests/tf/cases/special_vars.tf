;; special_vars.tf - TF's special variables: a flag or other enumerated
;; variable takes a name (any case) or its number and stores the name, and in
;; an expression it is dual-valued (the name when printed, the number in
;; arithmetic); a numeric variable takes the number at the start of its value;
;; a dtime keeps the form it was given. Defaults for sub/login/matching are TF's.
/eval /echo defaults sub=%sub login=%login matching=%matching ptime=%ptime
/set more=1
/eval /echo a=%more b=$[more] c=$[more+0] d=$[!more]
/set more=ON
/eval /echo e=%more
/set matching=2
/eval /echo f=%matching g=$[matching] h=$[matching*10]
/set max_trig=7abc
/eval /echo i=%max_trig
/set ptime=0:01
/eval /echo j=%ptime
/test more := 0
/eval /echo k=%more
/set flag1=off
/eval /echo l=$[flag1 ? 1 : 0]
/toggle more
/eval /echo m=%more
/quit
