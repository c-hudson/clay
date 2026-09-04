;; trigger.tf - /trigger runs text through the real trigger matcher (finding
;; B: Clay's /trigger today fires macros whose *pattern string* overlaps the
;; given text as a plain substring, rather than matching the text against
;; each trigger's glob/regexp pattern the way a real socket line would).
/def -t"hello*" -mglob greet = /echo matched %1
/trigger hello world
/def -t"^goodbye (.*)$" -mregexp bye = /echo bye-to=%P1
/trigger goodbye world
/trigger this matches nothing at all
/quit
