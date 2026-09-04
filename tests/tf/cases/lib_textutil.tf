;; requires-lib
;; lib_textutil.tf - TF's %| pipe operator. It is a List separator exactly
;; like %; (same precedence, left-to-right): a bare top-level line's %| is
;; never even parsed (top-level lines aren't split into a List at all - see
;; finding C.12), and within an /eval'd list, %| only pipes the ONE command
;; immediately before it, not everything since the start of the list - so
;; both commands piped through /uniq and /wc below are macros (dup3/words3)
;; whose OWN combined output is what gets connected to the next command's
;; input, per tf-help's "the output of a macro ... may be piped".
/require textutil.tf
/def dup3 = /echo x%;/echo x%;/echo y
/eval /dup3 %| /uniq
/def words3 = /echo a b c
/eval /words3 %| /wc -w
/quit
