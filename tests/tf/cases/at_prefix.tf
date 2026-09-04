;; at_prefix.tf - the "/@name" builtin-bypass prefix (finding C.6: Clay does
;; not parse "@" at all, so this whole case fails).
;;
;; This does NOT shadow a builtin with /def first (the task's original idea
;; of `/def echo = ...` then `/@echo`): every genuine tf core builtin (list,
;; def, undef, purge, quit, connect, bind, trigger, hook, dc, load, ...)
;; prints a "DEF: warning: macro \"x\" conflicts with the builtin command"
;; line the instant a macro of the same name is /def'd, and that warning
;; embeds this fixture's own absolute file path in tf's own diagnostic text -
;; which can never be a stable, reproducible target for Clay's output to
;; match. ("/def echo" specifically is worse: stdlib.tf already defines an
;; -i /echo macro, so the shadow calls itself and hits tf's recursion limit.)
;; So instead this shows /@ invoking a builtin directly, with no shadow
;; involved: /@purge takes an argument the same way plain /purge does
;; (finding C.4), bypassing the ordinary "greet" macro that's untouched.
/def greet = /echo via-macro
/greet
/def keep = /echo keep
/def drop = /echo drop
/@purge drop
/eval /echo drop=$[!!ismacro("drop")] keep=$[!!ismacro("keep")]
/quit
