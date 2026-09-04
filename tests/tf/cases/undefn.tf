;; undefn.tf - /def sets %? to the new macro's number (TF; not printed here,
;; only relied on), and /undefn removes a macro BY THAT NUMBER (finding B:
;; Clay's /undefn today takes a name pattern instead - see xfail.txt).
/def a = /echo a
/eval /undefn %?
/eval /echo ismacro=$[ismacro("a")]
/quit
