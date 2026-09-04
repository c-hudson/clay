;; for_syntax.tf - TF's own /for syntax: /for var min max command (finding
;; C.7: Clay parses the 4th token as a numeric step instead, so "/echo n=%i"
;; fails as an invalid step value).
/for i 1 3 /echo n=%i
/quit
