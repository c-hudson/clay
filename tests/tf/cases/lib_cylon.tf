;; requires-lib
;; lib_cylon.tf - $[strlen(cylon0)] as suggested (cylon.tf builds its color
;; strings from `@{...}` inline-attribute codes decoded via decode_attr(),
;; so its exact byte length is a real, deterministic property of the file).
/require cylon.tf
/eval /echo loaded=$[!!ismacro("cylon")] len=$[strlen(cylon0)]
/quit
