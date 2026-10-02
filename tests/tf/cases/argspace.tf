;; argspace.tf - a macro's argument text keeps its spacing: %*, %-N, %{-N}
;; and %-L slice the text as written, and /shift drops words without
;; re-joining what remains (verified against real tf).
/def f = /echo [%*] [%-1] [%{-1}] [%L] [%-L]%; /shift%; /echo [%*]
/f a   b    c
/def g = /echo [{*}] [{-1}]
/g x    y
/quit
