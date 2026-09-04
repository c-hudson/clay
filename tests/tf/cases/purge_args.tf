;; purge_args.tf - /purge with a target (name or -mglob pattern) should
;; remove only matching macros (finding C.4: Clay's /purge ignores its
;; arguments entirely and wipes every macro, regardless of what was asked
;; for). ismacro() results are normalized to 0/1 via !! since its raw value
;; is a macro's sequence number, which depends on how many library macros
;; were already loaded and so isn't portable between this fixture (no
;; library preload) and a run with one.
/def keep = /echo keep
/def drop = /echo drop
/purge drop
/keep
/eval /echo drop=$[!!ismacro("drop")] keep=$[!!ismacro("keep")]
/def a1 = /echo a1
/def a2 = /echo a2
/purge -mglob a*
/eval /echo $[!!ismacro("a1")]$[!!ismacro("a2")]$[!!ismacro("keep")]
/quit
