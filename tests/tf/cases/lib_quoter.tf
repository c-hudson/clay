;; requires-lib
/require quoter.tf
/eval /echo qdef=$[!!ismacro("qdef")] qfile=$[!!ismacro("qfile")] qtf=$[!!ismacro("qtf")] qsh=$[!!ismacro("qsh")]
/quit
