;; macros.tf - /def option flags Clay does not parse yet (finding C.1: -i -q
;; -I -T<type> -f -s all rejected), plus a nameless trigger macro (finding
;; C.9: nameless macros rejected) and a -1 one-shot trigger (already works).
/def -i inv = /echo inv
/inv
/def -q q = /echo q
/q
/def -Ttiny tt = /echo tt
/tt
/def -t"zzz*" = /echo anon
/trigger zzz
/def -1 -t"once*" o = /echo once
/trigger once
/trigger once
/quit
