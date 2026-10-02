;; pipes.tf - TF's "%|" pipe: in a list (a macro body, or /eval's text), the
;; command before "%|" has its output connected to the input (tfin) of the
;; command after it, which reads it with tfread() until it returns -1.
;; tfwrite() writes tfout like /echo does.
/def three = /echo one%; /echo two%; /echo three
/def count = /let n=0%; /while (tfread(line) >= 0) /test ++n%; /done%; /echo count=%n
/eval /three %| /count
/def upper = /while (tfread(l) >= 0) /echo $[toupper(l)]%; /done
/eval /three %| /upper
/def tw = /test tfwrite("written")
/eval /tw %| /count
/def chain = /three %| /upper %| /count
/chain
/quit
