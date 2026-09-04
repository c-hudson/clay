;; positional.tf - probes TinyFugue's positional-parameter substitutions in a
;; macro body: %{1-DEF} (arg 1, default DEF), %* (all args), %{#} (arg count),
;; %-1 (arg 1 through the last), %L (last arg), %-L (all but the last arg).
/def p = /echo one=%{1-DEF} all=%* count=%{#} rest=%-1 last=%L allbutlast=%-L
/p
/p a b c
/quit
