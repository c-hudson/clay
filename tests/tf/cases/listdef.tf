;; listdef.tf - /listdef is the standard library's `/def -i listdef = /list %*`: the
;; same listing as /list, with the same options and patterns. Clay has it natively,
;; so this case needs no library preload; real tf has the library loaded anyway.
;; Macro numbers differ (real tf's library defines hundreds first), so each check
;; lists exactly one macro and prints the line without its "% <number>: " prefix,
;; and whole listings are only compared with each other.
/def -i unnumbered = /let out=$(/eval %*)%; /echo $[substr(out, strchr(out, ":") + 2)]
/def ld_greet = /echo hello
/def -t"* waves" ld_wave = :waves back
/def -i ld_hidden = /echo hidden
/unnumbered /listdef ld_greet
/unnumbered /listdef -s -t ld_*
/unnumbered /listdef -s -I ld_*
/unnumbered /listdef -s ld_* = /echo hello
/unnumbered /listdef -s ld_w*
/def -i same_as_list = \
    /let a=$(/eval /listdef %*)%; \
    /let b=$(/eval /list %*)%; \
    /echo [%*] same=$[a =~ b] empty=$[a =~ ""]
/same_as_list -s ld_*
/same_as_list -s -i ld_*
/same_as_list ld_*
/same_as_list -s no_such_macro_*
/def -i nothing_listed = /listdef -s no_such_macro_*%; /echo nothing_listed_returns=%?
/nothing_listed
/quit
