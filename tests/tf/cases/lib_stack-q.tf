;; requires-lib
/require stack-q.tf
/push x
/push y
/eval /echo $(/pop) $(/pop)
/enqueue a
/enqueue b
/eval /echo $(/dequeue)
/quit
