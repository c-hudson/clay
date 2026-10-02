;; toplevel_nosub.tf - a top-level line in a loaded file runs exactly as
;; written: TF does not substitute %var, $[...] or $(...) there, only inside a
;; macro body or through /eval's own pass (finding C.12). A .tfrc line like
;; `/set time_format=%H:%M:%S` must store the format, not expand it.
/set time_format=%H:%M:%S
/eval /echo tf=%time_format
/set x=%{time_format}
/eval /echo x=[%x]
/def show = /echo show=[%{x}]
/show
/set y=$[1+2]
/eval /echo y=%y
/quit
