;; requires-lib
;; preload: stdlib.tf
;; lib_tintin.tf - loading tintin.tf always triggers one REDEF-hook warning
;; ("DEF: Redefined macro split") because both stdlib.tf and tintin.tf
;; define a macro named "split" - this is deterministic library content, not
;; an artifact of this probe, and (unlike an error naming *this fixture's*
;; own path - see at_prefix.tf/hooks.tf) the path it embeds is the *system*
;; tf-lib install location, which is the same fixed, portable path this
;; whole suite already depends on ($TFLIBDIR else /usr/share/tf5/tf-lib).
/require tintin.tf
/showme hi
/math x 2+3
/eval /echo $[x]
/variable y=z
/eval /echo $[y]
/quit
