;; Trailing space in a command is part of its argument (TF keeps it).
/set kprefix=>> 
/eval /echo [%kprefix]
/set foo   bar  
/eval /echo [%foo]
/def show = /echo [%*]
/show a b  
/quit
