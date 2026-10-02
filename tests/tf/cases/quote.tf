;; /quote -S: each source, <pre> and <suf>, what -s does to a TF command, that a
;; run (-dexec) line is never expanded, and the value a quote returns.
/set foo=bar
/quote -S -decho !echo a; echo b >&2; echo c
/quote -S -decho pre\!x !echo hi
/quote -S -decho !"echo hi" suf
/quote -S -decho !"echo \"q\"" end
/quote -S -decho `/echo [%{foo}]
/quote -S -soff -decho `/echo [%{foo}]
/quote -S -decho `"/echo a%;/echo b"suf
/quote -S /echo P: !echo hi
/def st = /quote -S -decho !exit 3%; /echo status=%?
/st
/def st3 = /quote -S -decho `/test 7%; /echo status=%?
/st3
/quote -S -decho !true
/quote -S -dexec !printf '/echo [%%{foo}]\n/echo x%%;/echo y\n'
/quit
